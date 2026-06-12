use std::process::Command;

#[test]
fn c_allocator_suite_passes_against_static_library() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let runner = format!("{manifest_dir}/tests/c/run-c-suite.sh");

    // The C suite script builds ralloc-staticlib which is now outside the workspace.
    // Skip until the C FFI crates are re-integrated.
    let staticlib_manifest = format!("{manifest_dir}/../ralloc_c_static/Cargo.toml");
    if !std::path::Path::new(&staticlib_manifest).exists() {
        eprintln!(
            "SKIP: ralloc_c_static not found at {} — C suite requires separate FFI crate build",
            staticlib_manifest
        );
        return;
    }

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
