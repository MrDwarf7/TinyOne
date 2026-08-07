use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use tinyone::{compile_source, run_program};

fn run_modes(source: &str) -> (String, String) {
    let mut vm = Vec::new();
    let mut jit = Vec::new();
    let program = compile_source(source).expect("compile");
    run_program(Arc::clone(&program), "vm", &mut vm, Vec::new()).expect("vm run");
    run_program(Arc::clone(&program), "jit", &mut jit, Vec::new()).expect("jit run");
    (
        String::from_utf8(vm).unwrap(),
        String::from_utf8(jit).unwrap(),
    )
}

fn assert_parity(source: &str, expected: &str) {
    let (vm, jit) = run_modes(source);
    assert_eq!(vm, expected, "vm output mismatch");
    assert_eq!(jit, expected, "jit output mismatch");
}

#[test]
fn vec_push_pop_round_trip_matches_backends() {
    let source = r#"
    let v = vec_new()
    let ignored = push(v, 1)
    let ignored2 = push(v, 2)
    let ignored3 = push(v, 3)
    print map_len(map_new())
    print len(v)
    print v[0]
    print v[1]
    print v[2]
    print pop(v)
    print len(v)
    print vec_clear(v)
    print len(v)
    "#;
    assert_parity(source, "0\n3\n1\n2\n3\n3\n2\n0\n0\n");
}

#[test]
fn map_basic_operations_match_backends() {
    let source = r#"
    let m = map_new()
    print map_set(m, 1, 100)
    print map_set(m, 2, 200)
    print map_set(m, 1, 111)
    print map_get(m, 1)
    print map_get(m, 2)
    print map_has(m, 1)
    print map_has(m, 99)
    print map_len(m)
    print map_del(m, 1)
    print map_len(m)
    print map_has(m, 1)
    "#;
    assert_parity(source, "100\n200\n111\n111\n200\n1\n0\n2\n1\n1\n0\n");
}

#[test]
fn map_keys_and_values_preserve_insertion_order() {
    let source = r#"
    let m = map_new()
    let ignored = map_set(m, 30, 1)
    let ignored2 = map_set(m, 10, 2)
    let ignored3 = map_set(m, 20, 3)
    let keys = map_keys(m)
    let values = map_values(m)
    print keys[0]
    print keys[1]
    print keys[2]
    print values[0]
    print values[1]
    print values[2]
    "#;
    assert_parity(source, "30\n10\n20\n1\n2\n3\n");
}

#[test]
fn io_capture_round_trips_writeln() {
    let source = r#"
    let captured = io_writeln(io_stdout(), "hi")
    let s = io_capture_stdout()
    print captured
    print str_byte_len(s)
    print str_char_len(s)
    "#;
    assert_parity(source, "3\n3\n3\n");
}

#[test]
fn string_byte_vs_char_indexing() {
    let source = r#"
    let text = "héllo"
    print str_byte_len(text)
    print str_char_len(text)
    print str_byte_at(text, 0)
    print str_char_at(text, 1)
    print str_slice(text, 1, 4)
    print str_is_utf8(text)
    "#;
    // 'h' is byte 104. é is multibyte. char_len = 5, byte_len = 6.
    assert_parity(source, "6\n5\n104\né\néll\n1\n");
}

#[test]
fn invalid_utf8_buffer_is_detected() {
    let source = r#"
    let mem = buffer(2)
    let p = ptr(mem, 0)
    let ignored = unsafe write8(p, 255)
    let ignored2 = unsafe write8(unsafe ptr_add(p, 1), 254)
    print str_is_utf8(mem)
    "#;
    assert_parity(source, "0\n");
}

#[test]
fn mutex_double_lock_errors() {
    let source = r#"
    let m = mutex_new()
    print mutex_lock(m)
    print mutex_unlock(m)
    print mutex_lock(m)
    print mutex_lock(m)
    "#;
    let mut vm = Vec::new();
    let err = run_program(
        compile_source(source).expect("compile"),
        "vm",
        &mut vm,
        Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("deadlock"), "got: {err}");
}

#[test]
fn atomic_add_overflows_at_i64_max() {
    let source = r#"
    let a = atomic_new(9223372036854775807)
    print atomic_load(a)
    print atomic_add(a, 1)
    "#;
    let mut vm = Vec::new();
    let err = run_program(
        compile_source(source).expect("compile"),
        "vm",
        &mut vm,
        Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("Memory_Overflow"), "got: {err}");
}

#[test]
fn result_round_trip_matches_backends() {
    let source = r#"
    let r = result_ok(42)
    print result_is_ok(r)
    print result_is_err(r)
    print result_unwrap(r)
    let e = result_err(99)
    print result_is_ok(e)
    print result_unwrap_err(e)
    "#;
    assert_parity(source, "1\n0\n42\n0\n99\n");
}

#[test]
fn option_unwrap_on_none_errors() {
    let source = r#"
    let s = option_some(7)
    print option_is_some(s)
    print option_unwrap(s)
    let n = option_none()
    print option_is_none(n)
    print option_unwrap(n)
    "#;
    let mut vm = Vec::new();
    let err = run_program(
        compile_source(source).expect("compile"),
        "vm",
        &mut vm,
        Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("None"), "got: {err}");
}

#[test]
fn typed_add_overflows_outside_range() {
    let source = r#"
    print typed_add(100, 28, "u8")
    print typed_add(200, 100, "u8")
    "#;
    let mut vm = Vec::new();
    let err = run_program(
        compile_source(source).expect("compile"),
        "vm",
        &mut vm,
        Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("Memory_Overflow"), "got: {err}");
}

#[test]
fn smallest_fit_and_promote_match_spec() {
    let source = r#"
    print smallest_fit(16)
    print smallest_fit(255)
    print smallest_fit(256)
    print smallest_fit(-16)
    print promote("i8", "u8")
    print promote("u8", "u16")
    print promote("i32", "i64")
    "#;
    assert_parity(source, "u8\nu8\nu16\ni8\ni16\nu32\ni64\n");
}

#[test]
fn type_of_recognizes_runtime_shapes() {
    let source = r#"
    print type_of(0)
    print type_of("abc")
    print type_of([1, 2])
    print type_of(alloc(1))
    print type_of(buffer(2))
    print type_of(map_new())
    print type_of(result_ok(1))
    print type_of(option_none())
    print type_of(mutex_new())
    print type_of(atomic_new(0))
    print type_of(null)
    "#;
    assert_parity(
        source,
        "i64\nString\nArray\nCell\nBuffer\nMap\nResult\nOption\nMutex\nAtomic\nNull\n",
    );
}

#[test]
fn scaffold_types_are_constructible_and_round_trip() {
    let source = r#"
    fn add(a, b) { return a + b }
    let closure = closure_new("add", [10, 20])
    print type_of(closure)
    print closure_function(closure)
    print len(closure_captures(closure))

    let bare = sum_new(3)
    print type_of(bare)
    print sum_tag(bare)
    print sum_has_payload(bare)
    let carried = sum_new(4, 99)
    print sum_unwrap(carried)

    let union = tagged_union_new(7, "payload")
    print tagged_union_tag(union)
    print tagged_union_unwrap(union)

    let dyn_value = dyn_new(12, 34, 55)
    print dyn_type_id(dyn_value)
    print dyn_vtable_id(dyn_value)
    print dyn_unwrap(dyn_value)

    let boxed = box_new(8)
    print box_get(boxed)
    print box_set(boxed, 9)
    print box_get(boxed)
    print type_of(char_new(65))
    print type_of(unsafe fd_new(3))
    print type_of(char_buffer_new([65, 66]))
    struct Pair { left, right }
    let pair = Pair(1, 2)
    print type_of(record_new(pair))
    let map = map_new()
    let ignored = map_set(map, "key", 4)
    print type_of(dictionary_new(map))
    let raw = unsafe alloc_new("i32", buffer(4))
    print type_of(raw)
    "#;
    assert_parity(
        source,
        "Closure\n0\n2\nSum\n3\n0\n99\n7\npayload\n12\n34\n55\n8\n9\n9\nChar\nFileDescriptor\nCharBuffer\nRecord\nDictionary\nAlloc\n",
    );
}

#[test]
fn math_constants_have_documented_values() {
    let source = r#"
    print math_const("PI_THOUSANDTHS")
    print math_const("E_THOUSANDTHS")
    print math_const("TAU_THOUSANDTHS")
    print math_const("MAX_I64")
    print math_const("MIN_I64")
    print math_abs(-42)
    print math_min(1, 2)
    print math_max(1, 2)
    print logic_and(1, 1)
    print logic_and(1, 0)
    print logic_or(0, 1)
    print logic_not(0)
    print logic_xor(1, 0)
    "#;
    assert_parity(
        source,
        "3142\n2718\n6283\n9223372036854775807\n-9223372036854775808\n42\n1\n2\n1\n0\n1\n1\n1\n",
    );
}

#[test]
fn sys_args_and_env_are_deterministic() {
    let source = r#"
    print sys_argc()
    print sys_argv(0)
    print sys_argv(1)
    print sys_env_has("FOO")
    print sys_env_get("FOO")
    print sys_env_has("MISSING")
    "#;
    let mut vm = Vec::new();
    let mut env = HashMap::new();
    env.insert("FOO".to_string(), "bar".to_string());
    tinyone::run_program_with_env(
        compile_source(source).expect("compile"),
        "vm",
        &mut vm,
        Vec::new(),
        vec!["program".to_string(), "alpha".to_string()],
        env,
    )
    .expect("run");
    let out = String::from_utf8(vm).unwrap();
    assert_eq!(out, "2\nprogram\nalpha\n1\nbar\n0\n");
}

#[test]
fn fs_read_write_round_trip_through_tempdir() {
    let dir = std::env::temp_dir().join(format!(
        "tinyone-stdlib-parity-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("hello.txt");
    let target_str = target.to_string_lossy().to_string();
    let source = format!(
        r#"
        let mem = buffer(4)
        let p = ptr(mem, 0)
        let ignored = unsafe write8(p, 104)
        let ignored2 = unsafe write8(unsafe ptr_add(p, 1), 105)
        let ignored3 = unsafe write8(unsafe ptr_add(p, 2), 33)
        let ignored4 = unsafe write8(unsafe ptr_add(p, 3), 10)
        let ignored5 = unsafe fs_write({path:?}, mem)
        let body = unsafe fs_read({path:?})
        print str_from_buffer(body)
        print fs_exists({path:?})
        let names = unsafe fs_list_dir({dir:?})
        print len(names)
        print path_basename({path:?})
        print path_dirname({path:?})
        print path_join("/tmp", "x")
        "#,
        path = target_str,
        dir = dir.to_string_lossy().to_string(),
    );
    let (vm, jit) = run_modes(&source);
    let expected = format!(
        "hi!\n\n1\n1\nhello.txt\n{}\n/tmp/x\n",
        dir.to_string_lossy()
    );
    assert_eq!(vm, expected, "vm output mismatch");
    assert_eq!(vm, jit);
    let _ = fs::remove_dir_all(&dir);
}

fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("{}", d.as_nanos()))
        .unwrap_or_else(|_| "x".to_string())
}
