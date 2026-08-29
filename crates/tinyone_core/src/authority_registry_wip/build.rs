// Recovered from recovery/project-shape-integration-20260829. This source is
// included only behind the permanently false Authority recovery gate.
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=TINYONE_TUF_INITIAL_ROOT");

    let root = match env::var_os("TINYONE_TUF_INITIAL_ROOT") {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            fs::read(&path).unwrap_or_else(|error| {
                panic!(
                    "could not read TINYONE_TUF_INITIAL_ROOT {}: {error}",
                    path.display()
                )
            })
        }
        None => Vec::new(),
    };

    let generated = format!("pub const EMBEDDED_TUF_INITIAL_ROOT: &[u8] = &{root:?};\n");
    let destination =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR for the build script"))
            .join("embedded_tuf_initial_root.rs");
    fs::write(destination, generated).expect("could not write embedded TUF root source");
}
