use std::process::Command;

#[test]
fn c_allocator_suite_passes_against_static_library() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let runner = format!("{manifest_dir}/tests/c/run-c-suite.sh");

    let output = Command::new("sh")
        .arg(&runner)
        .current_dir(manifest_dir)
        .output()
        .expect("C suite runner should execute");

    assert!(
        output.status.success(),
        "C suite failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
