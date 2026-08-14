use std::collections::HashMap;
use std::sync::Arc;

use tinyone::{compile_source, run_program_with_env, run_source};

#[test]
fn thread_join_collects_stdout_in_order() {
    // Threads print their args; output is queued until join, flushed before the last print
    let src = r#"
fn echo(n) {
  print n
  return n
}
let t1 = thread_spawn("echo", 1)
let t2 = thread_spawn("echo", 2)
let r1 = thread_join(t1)
let r2 = thread_join(t2)
print r1 + r2
"#;
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        run_source(src, mode, &mut out, Vec::new()).unwrap();
        let s = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = s.trim().lines().collect();
        // Last line must be 3 (r1 + r2 = 1 + 2)
        assert_eq!(
            lines.last(),
            Some(&"3"),
            "expected last line to be 3 in {mode}, got: {s:?}"
        );
        // All lines: the two thread prints and the final sum
        assert_eq!(
            lines.len(),
            3,
            "expected 3 lines total in {mode}, got: {s:?}"
        );
    }
}

#[test]
fn thread_join_twice_is_runtime_error() {
    let src = r#"
fn noop() { return 0 }
let t = thread_spawn("noop")
let r1 = thread_join(t)
let r2 = thread_join(t)
print 1
"#;
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        let result = run_source(src, mode, &mut out, Vec::new());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already joined"),
            "expected 'already joined' in {mode}: {msg}"
        );
    }
}

#[test]
fn thread_spawn_wrong_arity_errors() {
    // Arity is validated at spawn time; the thread_join is included so the
    // test catches the error whether the runtime checks eagerly or lazily.
    let src = r#"
fn add(a, b) { return a + b }
let t = thread_spawn("add", 1)
let r = thread_join(t)
"#;
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        let result = run_source(src, mode, &mut out, Vec::new());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("expects 2"),
            "expected arity error in {mode}: {msg}"
        );
    }
}

#[test]
fn mutex_protects_shared_counter() {
    // Two threads each increment a shared counter under mutex protection.
    // Final value must be exactly 2.
    let src = r#"
fn worker(m, counter) {
  let lk = mutex_lock(m)
  let old = atomic_load(counter)
  let st = atomic_store(counter, old + 1)
  let ul = mutex_unlock(m)
  return 0
}
let m = mutex_new()
let c = atomic_new(0)
let t1 = thread_spawn("worker", m, c)
let t2 = thread_spawn("worker", m, c)
let r1 = thread_join(t1)
let r2 = thread_join(t2)
print atomic_load(c)
"#;
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        run_source(src, mode, &mut out, Vec::new()).unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), "2", "{mode}");
    }
}

#[test]
fn atomic_add_is_sequentially_consistent() {
    let src = r#"
fn adder(a) {
  let res = atomic_add(a, 10)
  return 0
}
let a = atomic_new(0)
let t1 = thread_spawn("adder", a)
let t2 = thread_spawn("adder", a)
let r1 = thread_join(t1)
let r2 = thread_join(t2)
print atomic_load(a)
"#;
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        run_source(src, mode, &mut out, Vec::new()).unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), "20", "{mode}");
    }
}

#[test]
fn thread_spawn_unknown_function_errors() {
    let src = r#"let t = thread_spawn("does_not_exist")"#;
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        let result = run_source(src, mode, &mut out, Vec::new());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not found"),
            "expected 'not found' in {mode}: {msg}"
        );
    }
}

#[test]
fn thread_functions_read_a_snapshot_of_top_level_globals() {
    let src = r#"
let base = 40
fn worker() { return base + 2 }
let thread = thread_spawn("worker")
let result = thread_join(thread)
print result
"#;
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        run_source(src, mode, &mut out, Vec::new()).unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), "42", "{mode}");
    }
}

#[test]
fn thread_functions_inherit_system_arguments_and_environment() {
    let src = r#"
fn worker() {
  print sys_argc()
  print sys_argv(0)
  print sys_env_get("TINY_TEST")
  return 0
}
let thread = thread_spawn("worker")
let result = thread_join(thread)
"#;
    let program = compile_source(src).unwrap();
    for mode in ["vm", "jit"] {
        let mut out = Vec::new();
        let mut env = HashMap::new();
        env.insert("TINY_TEST".to_string(), "present".to_string());
        run_program_with_env(
            Arc::clone(&program),
            mode,
            &mut out,
            Vec::new(),
            vec!["argument".to_string()],
            env,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "1\nargument\npresent\n",
            "{mode}"
        );
    }
}
