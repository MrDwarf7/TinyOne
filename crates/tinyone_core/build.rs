// The Authority-backed build-script behavior is retained below for recovery,
// but is deliberately impossible to compile. Cargo must still see a normal
// build-script entry point for this workspace member.
#[cfg(all(feature = "authority-registry-wip", any()))]
mod authority_registry_wip {
    include!("src/authority_registry_wip/build.rs");
}

fn main() {
    println!("cargo:rerun-if-env-changed=TINYONE_TUF_INITIAL_ROOT");
}
