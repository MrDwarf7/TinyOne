use std::process::Command;

use tinyone_test_support::workspace_target_dir;

#[test]
fn release_build_emits_c_linkable_library_artifacts() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");

    let output = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(manifest_dir)
        .output()
        .expect("release build should execute");

    assert!(
        output.status.success(),
        "release build failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let release_dir = workspace_target_dir(manifest_dir, "release");
    let expected_static = if cfg!(windows) { "ralloc.lib" } else { "libralloc.a" };
    let expected_shared = if cfg!(windows) {
        "ralloc.dll"
    } else if cfg!(target_os = "macos") {
        "libralloc.dylib"
    } else {
        "libralloc.so"
    };
    assert!(
        release_dir.join(expected_static).is_file(),
        "missing C static library at {}",
        release_dir.join(expected_static).display()
    );
    assert!(
        release_dir.join(expected_shared).is_file(),
        "missing C shared library at {}",
        release_dir.join(expected_shared).display()
    );
}
