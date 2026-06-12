use std::fs;
use std::sync::Arc;

use tinyone::{Program, compile_file, compile_source, run_program, run_source};

fn run(source: &str, mode: &str) -> String {
    let mut out = Vec::new();
    run_source(source, mode, &mut out, Vec::new()).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn arithmetic_loops_and_functions() {
    let source = r#"
    fn mul_by_count(value, count) {
      let acc = 0
      while count > 0 {
        acc = acc + value
        count = count - 1
      }
      return acc
    }
    let i = 1
    let total = 0
    while i <= 8 {
      total = total + mul_by_count(i, 3)
      i = i + 1
    }
    print total
    "#;
    assert_eq!(run(source, "vm"), "108\n");
    assert_eq!(run(source, "jit"), "108\n");
}

#[test]
fn heap_pointers_and_buffers() {
    let source = r#"
    struct Pair { left, right }
    let values = [10, 20, 30]
    let second = ptr(values, 1)
    print unsafe ptr_load(second)
    print unsafe ptr_store(unsafe ptr_add(second, 1), 77)
    print values[2]
    let pair = Pair(4, 5)
    let field = fieldptr(pair, "right")
    print unsafe ptr_load(field)
    print unsafe ptr_store(field, 99)
    print pair.right
    let mem = buffer(8)
    let p = ptr(mem, 0)
    print unsafe write16(unsafe ptr_add(p, 2), 4660)
    print unsafe read8(unsafe ptr_add(p, 2))
    print unsafe read8(unsafe ptr_add(p, 3))
    "#;
    assert_eq!(run(source, "vm"), "20\n77\n77\n5\n99\n99\n4660\n52\n18\n");
}

#[test]
fn imports_and_artifact_roundtrip() {
    let root = std::env::temp_dir().join(format!("tinyone-rust-import-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("pairs.to"),
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
    .unwrap();
    let main_path = root.join("main.to");
    fs::write(
        &main_path,
        r#"
        import "pairs.to" as pairs
        let pair = pairs.Pair(18, 24)
        print pairs.sum_pair(pair)
        "#,
    )
    .unwrap();

    let program = compile_file(&main_path).unwrap();
    assert_eq!(program.modules.len(), 1);
    assert_eq!(program.modules[0].exported_functions, vec!["sum_pair"]);
    assert_eq!(program.modules[0].exported_structs, vec!["Pair"]);

    let loaded = Program::from_artifact(program.to_artifact()).unwrap();
    assert_eq!(program.fingerprint(), loaded.fingerprint());

    let mut out = Vec::new();
    run_program(Arc::new(loaded), "jit", &mut out, Vec::new()).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "42\n");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn conditionals_break_and_continue() {
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
    assert_eq!(run(source, "vm"), "25\n1\n");
    assert_eq!(run(source, "jit"), "25\n1\n");
}

#[test]
fn dynamic_array_push_and_pop_storage() {
    let source = r#"
    let values = []
    let i = 0
    while i < 4 {
      let ignored = push(values, i * 2)
      i = i + 1
    }
    print len(values)
    print values[2]
    print pop(values)
    print len(values)
    "#;
    assert_eq!(run(source, "vm"), "4\n4\n6\n3\n");
}

#[test]
fn loop_control_requires_loop_context() {
    let break_err = compile_source("break").unwrap_err().to_string();
    assert!(break_err.contains("Break outside loop"));
    let continue_err = compile_source("continue").unwrap_err().to_string();
    assert!(continue_err.contains("Continue outside loop"));
}

#[test]
fn pop_rejects_empty_arrays() {
    let err = run_source("let values = [] print pop(values)", "vm", &mut Vec::new(), Vec::new())
        .unwrap_err()
        .to_string();
    assert!(err.contains("empty array"));
}

#[test]
fn unsafe_gate_is_compile_time() {
    let err = compile_source("let values = [1] let p = ptr(values, 0) print ptr_load(p)")
        .unwrap_err()
        .to_string();
    assert!(err.contains("requires unsafe"));
}
