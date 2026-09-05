// Clippy pedantic: the CLI binary re-includes `mod cli` (also part of the lib), so it
// needs the same design-lint allows as the crate lib for `cli.rs`.
#![allow(
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]

mod cli;

fn main() {
    match cli::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("TinyLang error: {error}");
            std::process::exit(1);
        }
    }
}
