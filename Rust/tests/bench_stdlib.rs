//! Minimal Rust-side stdlib bench. Wired through `cargo test --test
//! bench_stdlib -- --nocapture` so it is easy to invoke from CI but does not
//! pollute `cargo test` output by default. The test always succeeds; the
//! values it prints are for human inspection.

use std::time::Instant;

use tinyone::{compile_source, run_program};

fn measure(label: &str, source: &str, iterations: usize) {
    let program = compile_source(source).expect("compile bench source");
    for mode in ["vm", "jit"] {
        let start = Instant::now();
        for _ in 0..iterations {
            let mut out = Vec::new();
            run_program(program.clone(), mode, &mut out, Vec::new()).expect("run");
        }
        let elapsed = start.elapsed();
        let per = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
        eprintln!("{label:32}  mode={mode:3}  {per:7.3} ms/iter");
    }
}

#[test]
fn stdlib_bench_smoke() {
    let iters = 20;
    measure(
        "vec_push_pop_1000",
        r#"
        let v = vec_new()
        let i = 0
        while i < 1000 {
          let ignored = push(v, i)
          i = i + 1
        }
        while len(v) > 0 {
          let ignored = pop(v)
        }
        print len(v)
        "#,
        iters,
    );
    measure(
        "map_set_get_100",
        r#"
        let m = map_new()
        let i = 0
        while i < 100 {
          let ignored = map_set(m, i, i * 2)
          i = i + 1
        }
        let total = 0
        let j = 0
        while j < 100 {
          total = total + map_get(m, j)
          j = j + 1
        }
        print total
        "#,
        iters,
    );
    measure(
        "typed_add_loop_1000",
        r#"
        let total = 0
        let i = 0
        while i < 1000 {
          total = typed_add(total, 1, "i32")
          i = i + 1
        }
        print total
        "#,
        iters,
    );
}
