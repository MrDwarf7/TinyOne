use std::{path::Path, process::Command};

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

    let release_dir = Path::new(manifest_dir).join("target/release");
    assert!(
        release_dir.join("libralloc.a").is_file(),
        "missing C static library at {}",
        release_dir.join("libralloc.a").display()
    );
    assert!(
        release_dir.join("libralloc.so").is_file(),
        "missing C shared library at {}",
        release_dir.join("libralloc.so").display()
    );
}
