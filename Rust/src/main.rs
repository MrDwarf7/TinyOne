mod cli;

fn main() {
    match cli::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("TinyOne error: {error}");
            std::process::exit(1);
        }
    }
}
