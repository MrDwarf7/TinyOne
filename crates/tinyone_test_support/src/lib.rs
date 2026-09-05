use std::path::{Path, PathBuf};

/// Returns the workspace root (repo root) by canonicalizing `CARGO_MANIFEST_DIR/../..`.
///
/// Canonicalization is attempted; on failure the un-canonicalized join is returned.
#[must_use]
pub fn repo_root_from_manifest(manifest_dir: &str) -> PathBuf {
    let manifest = Path::new(manifest_dir);
    manifest
        .join("../../")
        .canonicalize()
        .unwrap_or_else(|_| manifest.join("../../").clone())
}

/// Returns the workspace `target/<profile>` directory, preferring `CARGO_TARGET_DIR` if set.
///
/// Checks `CARGO_TARGET_DIR` env, then `manifest_dir/../../target/<profile>`,
/// then `manifest_dir/target/<profile>` (for non-workspace crates).
#[must_use]
pub fn workspace_target_dir(manifest_dir: &str, profile: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return Path::new(&dir).join(profile);
    }
    let manifest = Path::new(manifest_dir);
    let workspace = manifest.join("../../").join(format!("target/{profile}"));
    if workspace.exists() || !manifest.join(format!("target/{profile}")).exists() {
        workspace
    } else {
        manifest.join(format!("target/{profile}"))
    }
}

/// Asserts that a `Result` is `Err` and that the error string contains `needle`.
///
/// # Panics
/// If the result is `Ok(_)` or if the error string does not contain `needle`.
pub fn expect_error_contains<T, E: std::fmt::Display>(result: Result<T, E>, needle: &str) {
    let error = match result {
        Ok(_) => panic!("operation should fail"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains(needle), "expected error to contain {needle:?}, got {error:?}");
}
