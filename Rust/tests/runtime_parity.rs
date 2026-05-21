use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tinyone::{
    BytecodeVerifier, Function, Instr, JitCache, Op, Program, RuntimeValue, StructDef, TinyMemory,
    TinyOneError, TypeKind, compile_file, compile_source, load_artifact, run_program, run_program_report,
    write_artifact, write_jit_listing,
};

fn run_compiled(
    program: &Arc<Program>,
    mode: &str,
    inputs: Vec<&str>,
) -> Result<(String, Vec<RuntimeValue>), TinyOneError> {
    let mut stdout = Vec::new();
    let memory = run_program(
        Arc::clone(program),
        mode,
        &mut stdout,
        inputs.into_iter().map(ToOwned::to_owned).collect(),
    )?;
    Ok((
        String::from_utf8(stdout).expect("TinyOne output is UTF-8"),
        memory.snapshot(),
    ))
}

fn assert_backends_match(source: &str, expected_stdout: &str) -> Arc<Program> {
    let program = compile_source(source).expect("source should compile");
    let vm_result = run_compiled(&program, "vm", Vec::new()).expect("vm should run");
    let jit_result = run_compiled(&program, "jit", Vec::new()).expect("jit alias should run");
    assert_eq!(expected_stdout, vm_result.0);
    assert_eq!(vm_result, jit_result);
    program
}

fn assert_error_contains<T>(result: Result<T, TinyOneError>, needle: &str) {
    let error = match result {
        Ok(_) => panic!("operation should fail"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains(needle),
        "expected error to contain {needle:?}, got {error:?}"
    );
}

fn int(value: i64) -> RuntimeValue {
    RuntimeValue::I64(value)
}

fn minimal_program(code: Vec<Instr>) -> Program {
    Program {
        code,
        slot_count: 0,
        names: Vec::new(),
        functions: Vec::new(),
        strings: Vec::new(),
        structs: Vec::new(),
        fields: Vec::new(),
        modules: Vec::new(),
    }
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tinyone-rust-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp test dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn straightline_vm_and_jit_alias_match() {
    let source = r#"
    let a = 4
    let b = a * 5 + (6 - 2)
    let c = b / 3
    print b
    print c
    print b >= 24
    print c != 8
    "#;

    let program = assert_backends_match(source, "24\n8\n1\n0\n");
    assert!(
        program
            .code
            .iter()
            .all(|instr| !matches!(instr.op, Op::Jump | Op::JumpIfZero | Op::Call))
    );
}

#[test]
fn lexical_shadowing_restores_outer_scope() {
    let program = compile_file("tests/Programs/pass/003_variables_shadowing.to")
        .expect("shadowing fixture should compile");

    for mode in ["vm", "jit"] {
        assert_eq!(
            "20\n10\n15\n",
            run_compiled(&program, mode, Vec::new())
                .expect("shadowing fixture should run")
                .0
        );
    }
}

#[test]
fn loops_conditionals_and_loop_control_match() {
    let source = r#"
    let i = 0
    let total = 0
    while i < 10 {
      i = i + 1
      if i == 3 {
        continue
      }
      if i == 8 {
        break
      } else {
        total = total + i
      }
    }
    print total
    if total == 25 {
      print 1
    } else {
      print 0
    }
    "#;

    let program = assert_backends_match(source, "25\n1\n");
    assert!(
        program
            .code
            .iter()
            .any(|instr| matches!(instr.op, Op::Jump | Op::JumpIfZero))
    );
}

#[test]
fn expression_statements_else_if_and_boolean_ops_match() {
    let source = r#"
    let arr = []
    push(arr, 1)
    push(arr, 2)

    let label = 0
    if len(arr) == 0 {
      label = 99
    } else if len(arr) == 2 && !0 {
      label = 7
    } else {
      label = 3
    }

    print label
    print len(arr)
    if 0 || (label == 7 && len(arr) == 2) {
      print 1
    } else {
      print 0
    }
    if 1 || (1 / 0) {
      print 8
    }
    if 0 && (1 / 0) {
      print 0
    } else {
      print 9
    }
    "#;

    let program = assert_backends_match(source, "7\n2\n1\n8\n9\n");
    assert!(program.code.iter().any(|instr| instr.op == Op::Pop));
}

#[test]
fn low_level_ints_globals_and_unsafe_blocks_match() {
    let source = r#"
    let config = 7
    let mem = buffer(8)
    let base = ptr(mem, 0)

    unsafe {
      write8(base, u8(255))
      write16(ptr_add(base, 1), u16(513))
      write32(ptr_add(base, 3), u32(16909060))
      write8(ptr_add(ptr_add(base, 1), 2), u8(42))
    }

    fn byte_at_three() {
      unsafe {
        return read8(ptr_add(base, 3))
      }
    }

    fn config_value() {
      return config + 1
    }

    print type_of(u8(1))
    print type_of(u16(1))
    print type_of(u32(1))
    print unsafe read8(base)
    print unsafe read16(unsafe ptr_add(base, 1))
    print byte_at_three()
    print config_value()
    print type_of(unsafe read32(unsafe ptr_add(base, 3)))
    "#;

    let program = assert_backends_match(source, "u8\nu16\nu32\n255\n513\n42\n8\nu32\n");
    assert!(
        program
            .functions
            .iter()
            .flat_map(|function| function.code.iter())
            .any(|instr| instr.op == Op::LoadGlobal)
    );
}

#[test]
fn function_call_return_dispatch_matches() {
    let source = r#"
    fn mul_by_count(value, count) {
      let acc = 0
      while count > 0 {
        acc = acc + value
        count = count - 1
      }
      return acc
    }

    fn pair(x) {
      return mul_by_count(x, 2) + mul_by_count(x + 1, 3)
    }

    let i = 1
    let total = 0
    while i <= 8 {
      total = total + pair(i)
      i = i + 1
    }
    print total
    "#;

    let program = assert_backends_match(source, "204\n");
    assert_eq!(2, program.functions.len());
    assert!(program.code.iter().any(|instr| instr.op == Op::Call));
}

#[test]
fn nested_control_flow_transfers_match() {
    let source = r#"
    let i = 0
    let marker = 1
    let trips = 0
    while i < 10 {
      let gate = 1
      while gate {
        trips = trips + marker
        gate = 0
      }
      marker = marker + 1
      i = i + 1
    }
    print trips
    "#;

    assert_backends_match(source, "55\n");
}

#[test]
fn runtime_division_errors_match() {
    let program = compile_source(
        r#"
        let zero = 0
        print 12 / zero
        "#,
    )
    .expect("source should compile");

    for mode in ["vm", "jit"] {
        assert_error_contains(run_compiled(&program, mode, Vec::new()), "Division by zero");
    }
}

#[test]
fn jit_cache_reuses_straightline_dispatch_and_heap_programs() {
    let cases = [
        (
            "straightline",
            r#"
            let x = 40 + 2
            print x
            "#,
        ),
        (
            "dispatch",
            r#"
            fn inc(value) {
              return value + 1
            }
            let i = 0
            while i < 4 {
              i = inc(i)
            }
            print i
            "#,
        ),
        (
            "heap",
            r#"
            struct Pair { left, right }
            let values = [1, 2, 3]
            let pair = Pair(values[0], len(values))
            print pair.left + pair.right
            "#,
        ),
    ];

    for (name, source) in cases {
        let program = compile_source(source).expect("source should compile");
        let mut cache = JitCache::new();

        assert!(cache.is_empty(), "{name}");
        let first = cache.compile(&*program).expect("jit compile") as *const _;
        assert_eq!(1, cache.len(), "{name}");

        let second = cache.compile(&*program).expect("jit compile") as *const _;
        assert_eq!(first, second, "{name}");
        assert_eq!(1, cache.len(), "{name}");
    }
}

#[test]
fn jit_compiles_to_lowered_bytecode_listing() {
    let program = compile_source(
        r#"
        let x = 40 + 2
        print x
        "#,
    )
    .expect("source should compile");
    let mut cache = JitCache::new();
    let compiled = cache.compile(&*program).expect("jit compile");

    assert_eq!(program.fingerprint(), compiled.fingerprint());
    assert!(compiled.listing().contains(".chunk 0 main"));
    assert!(compiled.listing().contains("store.i 0"));
    let stats = cache.stats();
    assert_eq!(1, stats.programs);
    assert_eq!(1, stats.compiled_chunks);
    assert!(stats.compiled_ops > 0);
}

#[test]
fn write_jit_listing_emits_inspectable_file() {
    let program = compile_source("let x = 6 * 7 print x").expect("source should compile");
    let temp = TestDir::new("jit-listing");
    let path = temp.path().join("program.tjit");

    write_jit_listing(&*program, &path).expect("write jit listing");
    let listing = fs::read_to_string(path).expect("read jit listing");

    assert!(listing.contains("tinyone adaptive-jit"));
    assert!(listing.contains(".chunk 0 main"));
    assert!(listing.contains("store.i 0 42"));
}

#[test]
fn jit_quickens_hot_back_edges_after_warm_runs() {
    let program = compile_source(
        r#"
        let i = 0
        let total = 0
        while i < 64 {
          total = total + i
          i = i + 1
        }
        print total
        "#,
    )
    .expect("source should compile");
    let mut cache = JitCache::new();

    for _ in 0..2 {
        let mut stdout = Vec::new();
        cache
            .run_program(&*program, &mut stdout, Vec::new())
            .expect("jit should run");
        assert_eq!("2016\n", String::from_utf8(stdout).expect("UTF-8 output"));
    }

    let stats = cache.stats();
    assert!(stats.hot_back_edges >= 8);
    assert!(stats.hot_ranges >= 1);
    assert!(stats.quickened_ops > 0);

    let listing = cache.compile(&*program).expect("jit compile").listing();
    assert!(listing.contains("add.int"));
    assert!(listing.contains("jmp.hot"));
}

#[test]
fn jit_cache_run_source_supports_warm_api_path() {
    let source = r#"
    let i = 0
    let total = 0
    while i < 8 {
      total = total + i
      i = i + 1
    }
    print total
    "#;
    let mut cache = JitCache::new();

    for _ in 0..2 {
        let mut stdout = Vec::new();
        let memory = cache
            .run_source(source, &mut stdout, Vec::new())
            .expect("warm jit source should run");
        assert_eq!("28\n", String::from_utf8(stdout).expect("UTF-8 output"));
        assert_eq!(2, memory.snapshot().len());
        assert_eq!(1, cache.len());
    }
}

#[test]
fn heap_arrays_structs_strings_fields_and_dynamic_storage_match() {
    let source = r#"
    struct Point { x, y }
    let values = [10, 20, 30]
    set values[1] = 99
    print push(values, 40)
    let p = Point(values[1], len(values))
    set p.y = p.y + 1
    let msg = "hi"
    print msg
    print values
    print pop(values)
    print len(values)
    print values[1]
    print p.x
    print p.y
    print len(msg)
    print msg[1]
    "#;

    assert_backends_match(source, "4\nhi\n[10, 99, 30, 40]\n40\n3\n99\n99\n5\n2\ni\n");
}

#[test]
fn utf8_string_literals_are_preserved() {
    assert_backends_match(
        r#"
        let text = "é"
        print text
        print len(text)
        "#,
        "é\n1\n",
    );
}

#[test]
fn pointer_cells_and_deterministic_input_match() {
    let source = r#"
    let start = read_int()
    let ptr = alloc(start)
    print load(ptr)
    print store(ptr, load(ptr) + 5)
    print load(ptr)
    let done = unsafe free(ptr)
    "#;
    let program = compile_source(source).expect("source should compile");

    for mode in ["vm", "jit"] {
        let (stdout, memory) =
            run_compiled(&program, mode, vec!["37"]).expect("program should run");
        assert_eq!("37\n42\n42\n", stdout);
        assert_eq!(3, memory.len());
    }
}

#[test]
fn input_exhaustion_errors_on_explicit_reads() {
    let program = compile_file("tests/Programs/fail_runtime/008_input_exhaustion.to")
        .expect("input exhaustion fixture should compile");

    for mode in ["vm", "jit"] {
        assert_error_contains(run_compiled(&program, mode, Vec::new()), "Input exhausted");
    }

    let exact = compile_source(
        r#"
        print read()
        print read_int()
        print read_str()
        "#,
    )
    .expect("input source should compile");
    for mode in ["vm", "jit"] {
        assert_eq!(
            "12\n34\nhello\n",
            run_compiled(&exact, mode, vec!["12", "34", "hello"])
                .expect("exact input queue should run")
                .0
        );
    }
}

#[test]
fn manual_free_requires_unsafe_and_keeps_failures_contained() {
    assert_error_contains(
        compile_source("let values = [1] let ignored = free(values)"),
        "requires unsafe",
    );

    let program = compile_source(
        r#"
        let values = [1]
        let ignored = unsafe free(values)
        print values[0]
        "#,
    )
    .expect("source should compile");

    for mode in ["vm", "jit"] {
        assert_error_contains(run_compiled(&program, mode, Vec::new()), "Use after free");
    }
}

#[test]
fn raw_pointers_require_unsafe_and_match_backends() {
    let source = r#"
    struct Pair { left, right }
    let values = [10, 20, 30]
    let second = ptr(values, 1)
    print ptr_addr(second)
    print ptr_type(second)
    print unsafe ptr_load(second)
    print unsafe ptr_store(unsafe ptr_add(second, 1), 77)
    print values[2]
    let pair = Pair(4, 5)
    let field = fieldptr(pair, "right")
    print unsafe ptr_load(field)
    print unsafe ptr_store(field, 99)
    print pair.right
    let cell = alloc(12)
    let raw = ptr(cell)
    print unsafe ptr_load(raw)
    print unsafe ptr_store(raw, 13)
    print load(cell)
    let ptr_cell = alloc(second)
    print ptr_type(load(ptr_cell))
    "#;

    assert_backends_match(
        source,
        "1\narray\n20\n77\n77\n5\n99\n99\n12\n13\n13\narray\n",
    );
    assert_error_contains(
        compile_source("let values = [1] let p = ptr(values, 0) print ptr_load(p)"),
        "requires unsafe",
    );
}

#[test]
fn raw_pointer_arithmetic_checks_runtime_bounds() {
    let program = compile_source(
        r#"
        let values = [1]
        let p = ptr(values, 0)
        print unsafe ptr_load(unsafe ptr_add(p, 2))
        "#,
    )
    .expect("source should compile");

    for mode in ["vm", "jit"] {
        assert_error_contains(run_compiled(&program, mode, Vec::new()), "out of bounds");
    }
}

#[test]
fn pointer_arithmetic_overflow_reports_runtime_error() {
    let program = compile_source(
        r#"
        let mem = buffer(1)
        let p = ptr(mem, 0)
        let huge = unsafe ptr_add(p, 9223372036854775807)
        print ptr_offset(unsafe ptr_add(huge, 1))
        "#,
    )
    .expect("source should compile");

    for mode in ["vm", "jit"] {
        assert_error_contains(run_compiled(&program, mode, Vec::new()), "offset overflow");
    }
}

#[test]
fn null_metadata_buffers_and_sized_memory_match() {
    let source = r#"
    struct Pair { left, right }
    let nothing = null
    print is_null(nothing)

    let values = [1, 2]
    let item = ptr(values, 1)
    print ptr_kind(item)
    print ptr_offset(item)
    set values[1] = 9
    print unsafe ptr_load(item)

    let pair = Pair(4, 5)
    let field = fieldptr(pair, "right")
    print ptr_kind(field)
    print ptr_field(field)
    set pair.right = 11
    print unsafe ptr_load(field)

    let mem = buffer(16)
    let p = ptr(mem, 0)
    print is_null(p)
    print ptr_eq(p, ptr(mem, 0))
    print ptr_ne(p, nothing)
    print ptr_base(p) > 0
    print ptr_offset(unsafe ptr_add(p, 3))
    print ptr_kind(p)
    print len(ptr_field(p))
    print ptr_type(cast_ptr(p, "i32"))
    print ptr_eq(cast_ptr(p, "i32"), p)
    print unsafe read8(p)
    print unsafe write8(unsafe ptr_add(p, 1), 255)
    print unsafe read8(unsafe ptr_add(p, 1))
    print unsafe write16(unsafe ptr_add(p, 2), 4660)
    print unsafe read8(unsafe ptr_add(p, 2))
    print unsafe read8(unsafe ptr_add(p, 3))
    print unsafe read16(unsafe ptr_add(p, 2))
    print unsafe write32(unsafe ptr_add(p, 4), 305419896)
    print unsafe read32(unsafe ptr_add(p, 4))
    "#;

    assert_backends_match(
        source,
        concat!(
            "1\narray\n1\n9\nfield\nright\n11\n0\n1\n1\n1\n3\n",
            "buffer\n0\ni32\n1\n0\n255\n255\n4660\n52\n18\n4660\n",
            "305419896\n305419896\n"
        ),
    );
}

#[test]
fn raw_memory_operations_require_unsafe_and_check_bounds() {
    assert_error_contains(
        compile_source("let mem = buffer(1) let p = ptr(mem, 0) print read8(p)"),
        "requires unsafe",
    );

    let programs = [
        compile_source("let mem = buffer(1) let p = ptr(mem, 0) print unsafe read16(p)")
            .expect("source should compile"),
        compile_source("let mem = buffer(1) let p = ptr(mem, 0) print unsafe write8(p, 256)")
            .expect("source should compile"),
    ];

    for program in programs {
        for mode in ["vm", "jit"] {
            let error = run_compiled(&program, mode, Vec::new())
                .expect_err("program should fail")
                .to_string();
            assert!(
                error.contains("out of bounds") || error.contains("range"),
                "unexpected error: {error}"
            );
        }
    }
}

#[test]
fn dynamic_allocations_are_bounded_before_host_allocation() {
    let programs = [
        compile_source("print len(array(1000001, 0))").expect("source should compile"),
        compile_source("print len(buffer(16777217))").expect("source should compile"),
        compile_source(
            r#"
            let a = buffer(1048576)
            let b = buffer(1048576)
            let c = buffer(1048576)
            let d = buffer(1048576)
            let e = buffer(1)
            print len(e)
            "#,
        )
        .expect("source should compile"),
    ];

    for program in programs {
        for mode in ["vm", "jit"] {
            let error = run_compiled(&program, mode, Vec::new())
                .expect_err("program should fail")
                .to_string();
            assert!(
                error.contains("exceeds maximum") || error.contains("Heap byte limit"),
                "unexpected error: {error}"
            );
        }
    }
}

#[test]
fn runtime_owned_heap_is_released_on_shutdown_without_gc() {
    let program = compile_source(
        r#"
        struct Pair { left, right }
        let values = [1, 2, 3]
        let pair = Pair(values, buffer(64))
        let cell = alloc(pair)
        print len(values) + len(pair.right)
        "#,
    )
    .expect("source should compile");

    for mode in ["vm", "jit"] {
        let mut stdout = Vec::new();
        let report = run_program_report(Arc::clone(&program), mode, &mut stdout, Vec::new())
            .expect("program should run");
        assert_eq!("67\n", String::from_utf8(stdout).expect("UTF-8 output"));
        assert!(report.heap_before_shutdown.live_objects >= 4);
        assert!(report.heap_before_shutdown.live_bytes > 0);
        assert_eq!(0, report.heap_after_shutdown.live_objects);
        assert_eq!(0, report.heap_after_shutdown.live_bytes);
        assert_eq!(
            report.heap_before_shutdown.live_objects as u64,
            report.heap_after_shutdown.shutdown_frees
        );
    }
}

#[test]
fn derived_pointers_fail_after_base_free_even_if_address_is_reused() {
    let programs = [
        compile_source(
            r#"
            let values = [1, 2]
            let p = ptr(values, 1)
            let ignored = unsafe free(values)
            let replacement = [7, 8]
            print unsafe ptr_load(p)
            "#,
        )
        .expect("source should compile"),
        compile_source(
            r#"
            let values = [1, 2]
            let p = ptr(values, 1)
            let ignored = unsafe free(values)
            let replacement = [7, 8]
            print ptr_kind(p)
            "#,
        )
        .expect("source should compile"),
        compile_source(
            r#"
            struct Pair { left, right }
            let pair = Pair(1, 2)
            let p = fieldptr(pair, "right")
            let ignored = unsafe free(pair)
            let replacement = Pair(3, 4)
            print unsafe ptr_load(p)
            "#,
        )
        .expect("source should compile"),
    ];

    for program in programs {
        for mode in ["vm", "jit"] {
            let error = run_compiled(&program, mode, Vec::new())
                .expect_err("program should fail")
                .to_string();
            assert!(
                error.contains("Stale heap pointer") || error.contains("Use after free"),
                "unexpected error: {error}"
            );
        }
    }
}

#[test]
fn recursive_calls_stop_at_language_call_depth_limit() {
    let program = compile_source(
        r#"
        fn recurse(value) {
          return recurse(value + 1)
        }
        print recurse(0)
        "#,
    )
    .expect("source should compile");

    for mode in ["vm", "jit"] {
        assert_error_contains(
            run_compiled(&program, mode, Vec::new()),
            "Call stack overflow",
        );
    }
}

#[test]
fn imports_and_artifact_roundtrip() {
    let temp = TestDir::new("artifact");
    fs::write(
        temp.path().join("pairs.to"),
        r#"
        fn hidden(p) {
          return p.left + p.right + 1000
        }

        export struct Pair { left, right }

        export fn sum_pair(p) {
          return p.left + p.right
        }
        "#,
    )
    .expect("write module");

    let main_path = temp.path().join("main.to");
    fs::write(
        &main_path,
        r#"
        import "pairs.to" as pairs
        let pair = pairs.Pair(18, 24)
        print pairs.sum_pair(pair)
        "#,
    )
    .expect("write main");

    let program = compile_file(&main_path).expect("compile file");
    assert_eq!(1, program.modules.len());
    assert_eq!("pairs", program.modules[0].path);
    assert_eq!(
        vec!["sum_pair".to_string()],
        program.modules[0].exported_functions
    );
    assert_eq!(
        vec!["Pair".to_string()],
        program.modules[0].exported_structs
    );
    assert!(
        !program
            .to_artifact()
            .to_string()
            .contains(&temp.path().display().to_string()),
        "artifact should not contain canonical temp paths"
    );

    let artifact_path = temp.path().join("main.tobc.json");
    write_artifact(&*program, &artifact_path).expect("write artifact");
    let loaded = Arc::new(load_artifact(&artifact_path).expect("load artifact"));
    assert_eq!(program.fingerprint(), loaded.fingerprint());

    for mode in ["vm", "jit"] {
        assert_eq!(
            "42\n",
            run_compiled(&loaded, mode, Vec::new())
                .expect("run loaded")
                .0
        );
    }
}

#[test]
fn import_manifest_namespaces_and_export_visibility() {
    let temp = TestDir::new("manifest");
    fs::write(
        temp.path().join("tinyone.json"),
        r#"{"package": "demo", "modules": {"math": "pkg/math.to"}}"#,
    )
    .expect("write manifest");
    fs::create_dir(temp.path().join("pkg")).expect("create pkg dir");
    fs::write(
        temp.path().join("pkg").join("math.to"),
        r#"
        fn hidden(value) {
          return value + 100
        }

        fn sum_pair(p) {
          return p.left + p.right + hidden(0)
        }

        export struct Pair { left, right }

        export fn exported_sum(p) {
          return sum_pair(p)
        }
        "#,
    )
    .expect("write module");

    let main_path = temp.path().join("main.to");
    fs::write(
        &main_path,
        r#"
        import "math" as m
        let pair = m.Pair(10, 20)
        print m.exported_sum(pair)
        "#,
    )
    .expect("write main");

    for mode in ["vm", "jit"] {
        let program = compile_file(&main_path).expect("compile file");
        assert_eq!(
            "130\n",
            run_compiled(&program, mode, Vec::new()).expect("run").0
        );
    }

    let bad_unqualified = temp.path().join("bad_unqualified.to");
    fs::write(
        &bad_unqualified,
        r#"
        import "math" as m
        let pair = Pair(10, 20)
        print m.exported_sum(pair)
        "#,
    )
    .expect("write bad file");
    assert_error_contains(
        compile_file(&bad_unqualified),
        "Undefined function or constructor",
    );

    let bad_private = temp.path().join("bad_private.to");
    fs::write(
        &bad_private,
        r#"
        import "math" as m
        print m.hidden(1)
        "#,
    )
    .expect("write bad file");
    assert_error_contains(compile_file(&bad_private), "not exported");
}

#[test]
fn block_scope_hides_loop_locals_and_loop_control_requires_loop() {
    assert_error_contains(
        compile_source(
            r#"
            let i = 0
            while i < 1 {
              let scoped = 9
              i = i + 1
            }
            print scoped
            "#,
        ),
        "Undefined variable",
    );
    assert_error_contains(compile_source("break"), "Break outside loop");
    assert_error_contains(compile_source("continue"), "Continue outside loop");
}

#[test]
fn compile_diagnostics_include_line_column_and_span() {
    let error = compile_source("let x = 1\nprint missing\n")
        .expect_err("source should fail")
        .to_string();

    assert!(error.contains("<source>:2:7"), "{error}");
    assert!(error.contains("Undefined variable"), "{error}");
    assert!(error.contains("^"), "{error}");
}

#[test]
fn pop_rejects_empty_arrays() {
    let program =
        compile_source("let values = [] print pop(values)").expect("source should compile");
    for mode in ["vm", "jit"] {
        assert_error_contains(run_compiled(&program, mode, Vec::new()), "empty array");
    }
}

#[test]
fn memory_allocation_reset_and_bounds() {
    let mut memory = TinyMemory::new(3);
    assert_eq!(vec![int(0), int(0), int(0)], memory.snapshot());

    memory.store(1, int(99)).expect("store slot");
    assert_eq!(int(99), memory.load(1).expect("load slot"));
    assert_eq!(vec![int(0), int(99), int(0)], memory.snapshot());

    memory.reset();
    assert_eq!(vec![int(0), int(0), int(0)], memory.snapshot());

    assert_error_contains(memory.load(3), "Invalid memory slot");
    assert_error_contains(memory.store(3, int(1)), "Invalid memory slot");
}

#[test]
fn verifier_rejects_stack_underflow_before_runtime() {
    let program = minimal_program(vec![
        Instr::new(Op::Print, 0, 0),
        Instr::new(Op::Halt, 0, 0),
    ]);
    assert_error_contains(BytecodeVerifier::verify(&program), "stack underflow");
}

#[test]
fn verifier_rejects_invalid_jump_target() {
    let program = minimal_program(vec![
        Instr::new(Op::PushInt, 1, 0),
        Instr::new(Op::JumpIfZero, 99, 0),
        Instr::new(Op::Halt, 0, 0),
    ]);
    assert_error_contains(BytecodeVerifier::verify(&program), "targets 99");
}

#[test]
fn verifier_rejects_call_arity_mismatch() {
    let function = Function {
        name: "id".to_string(),
        param_count: 1,
        code: vec![Instr::new(Op::Load, 0, 0), Instr::new(Op::Return, 0, 0)],
        slot_count: 1,
        names: vec!["value".to_string()],
    };
    let mut program = minimal_program(vec![
        Instr::new(Op::PushInt, 7, 0),
        Instr::new(Op::Call, 0, 0),
        Instr::new(Op::Print, 0, 0),
        Instr::new(Op::Halt, 0, 0),
    ]);
    program.functions.push(function);

    assert_error_contains(BytecodeVerifier::verify(&program), "expects 1 argument");
}

#[test]
fn verifier_rejects_invalid_slot_and_struct_arity() {
    let mut invalid_slot = minimal_program(vec![
        Instr::new(Op::Load, 2, 0),
        Instr::new(Op::Print, 0, 0),
        Instr::new(Op::Halt, 0, 0),
    ]);
    invalid_slot.slot_count = 1;
    invalid_slot.names.push("only".to_string());
    assert_error_contains(BytecodeVerifier::verify(&invalid_slot), "invalid slot 2");

    let mut invalid_struct = minimal_program(vec![
        Instr::new(Op::PushInt, 1, 0),
        Instr::new(Op::MakeStruct, 0, 1),
        Instr::new(Op::Print, 0, 0),
        Instr::new(Op::Halt, 0, 0),
    ]);
    invalid_struct.structs.push(StructDef {
        name: "Pair".to_string(),
        fields: vec!["left".to_string(), "right".to_string()],
    });
    assert_error_contains(
        BytecodeVerifier::verify(&invalid_struct),
        "expects 2 field value",
    );
}

#[test]
fn all_43_runtime_value_variants_are_representable() {
    let _unit      = RuntimeValue::Unit;
    let _bool_t    = RuntimeValue::Bool(true);
    let _i8        = RuntimeValue::I8(0i8);
    let _i16       = RuntimeValue::I16(0i16);
    let _i32       = RuntimeValue::I32(0i32);
    let _i64       = RuntimeValue::I64(0i64);
    let _u8        = RuntimeValue::U8(0u8);
    let _u16       = RuntimeValue::U16(0u16);
    let _u32       = RuntimeValue::U32(0u32);
    let _u64       = RuntimeValue::U64(0u64);
    let _bf16      = RuntimeValue::Bf16(0u16);
    let _fp16      = RuntimeValue::Float { kind: TypeKind::Fp16, bits: 0.0 };
    let _fp32      = RuntimeValue::Float { kind: TypeKind::Fp32, bits: 0.0 };
    let _fp64      = RuntimeValue::Float { kind: TypeKind::Fp64, bits: 0.0 };
    let _null      = RuntimeValue::Null;
    let _func      = RuntimeValue::Function(0u32);
    let _phantom   = RuntimeValue::Phantom;
    let _zst       = RuntimeValue::Zst(TypeKind::Zst);
    let _unsafe_v  = RuntimeValue::Unsafe;
}
