use std::env;
use std::io::{self, Read};

use tinyone::{
    CompileCacheStatus, JitOptions, TinyOneError, compile_file_cached_verified_with_options,
    compile_file_unoptimized_verified, compile_file_verified, load_verified_artifact,
    run_verified_program_with_jit_options, write_artifact, write_binary_artifact,
    write_jit_listing,
};

#[derive(Debug)]
struct Args {
    path: Option<String>,
    mode: String,
    check: bool,
    emit_bytecode: Option<String>,
    emit_jit: Option<String>,
    run_bytecode: Option<String>,
    inputs: Vec<String>,
    stdin: bool,
    verbose: bool,
    optimize: bool,
    cache: bool,
    jit_threshold: u16,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            path: None,
            mode: "jit".to_string(),
            check: false,
            emit_bytecode: None,
            emit_jit: None,
            run_bytecode: None,
            inputs: Vec::new(),
            stdin: false,
            verbose: false,
            optimize: true,
            cache: true,
            jit_threshold: tinyone::DEFAULT_HOT_BACK_EDGE_THRESHOLD,
            help: false,
        }
    }
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    let mut iter = argv.into_iter();
    let _ = iter.next();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => args.help = true,
            "--mode" => {
                args.mode = iter.next().ok_or("--mode requires a value")?;
                if args.mode != "jit" && args.mode != "vm" {
                    return Err("--mode must be 'jit' or 'vm'".to_string());
                }
            }
            "-j" | "--jit" => args.mode = "jit".to_string(),
            "--vm" => args.mode = "vm".to_string(),
            "-O0" | "--no-optimize" => args.optimize = false,
            "-O1" | "--optimize" => args.optimize = true,
            "--no-cache" => args.cache = false,
            "--no-jit-quickening" => args.jit_threshold = 0,
            "--jit-threshold" => {
                let value = iter.next().ok_or("--jit-threshold requires a value")?;
                args.jit_threshold = value.parse::<u16>().map_err(|_| {
                    "--jit-threshold must be an integer from 0 to 65535".to_string()
                })?;
            }
            "--check" => args.check = true,
            "--emit-bytecode" => {
                args.emit_bytecode = Some(iter.next().ok_or("--emit-bytecode requires a path")?);
            }
            "--emit-jit" => {
                args.emit_jit = Some(iter.next().ok_or("--emit-jit requires a path")?);
            }
            "--run-bytecode" => {
                args.run_bytecode = Some(iter.next().ok_or("--run-bytecode requires a path")?);
            }
            "--input" => {
                args.inputs
                    .push(iter.next().ok_or("--input requires a value")?);
            }
            "--stdin" => args.stdin = true,
            "--verbose" => args.verbose = true,
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => {
                if args.path.is_some() {
                    return Err(format!("unexpected extra path {arg}"));
                }
                args.path = Some(arg);
            }
        }
    }
    Ok(args)
}

fn print_help() {
    println!("usage: tinylang [OPTIONS] [path]");
    println!();
    println!("Options:");
    println!("  --mode {{jit,vm}}        Execution mode (default: jit)");
    println!("  -j, --jit               Use adaptive JIT mode");
    println!("  --vm                    Use portable VM mode");
    println!("  -O0, --no-optimize      Disable bytecode optimization");
    println!("  -O1, --optimize         Enable bytecode optimization (default)");
    println!("  --no-cache              Disable the dependency-validated disk compile cache");
    println!("  --jit-threshold N       Quicken loops after N back edges (default: 8)");
    println!("  --no-jit-quickening     Disable adaptive JIT quickening");
    println!("  --check                 Compile only, do not run");
    println!("  --emit-bytecode PATH    Write JSON, or compact binary when PATH ends in .tob");
    println!("  --emit-jit PATH         Write a JIT listing to PATH");
    println!("  --run-bytecode PATH     Run a compiled bytecode artifact");
    println!("  --input VALUE           Supply a program input value (repeatable)");
    println!("  --stdin                 Read input values from stdin");
    println!("  --verbose               Print program metadata before running");
    println!("  -h, --help              Show this help message");
}

pub(crate) fn run() -> Result<i32, TinyOneError> {
    let mut args = parse_args(env::args()).map_err(TinyOneError::Compile)?;
    if args.help {
        print_help();
        return Ok(0);
    }
    if args.stdin {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| TinyOneError::Compile(format!("File error: {error}")))?;
        args.inputs.extend(text.lines().map(str::to_string));
    }

    let mut cache_status = None;
    let program = if let Some(path) = args.run_bytecode {
        load_verified_artifact(path)?
    } else {
        let Some(path) = args.path else {
            return Err(TinyOneError::Compile(
                "File error: a source path is required".to_string(),
            ));
        };
        if args.cache {
            let (program, status) = compile_file_cached_verified_with_options(path, args.optimize)?;
            cache_status = Some(status);
            program
        } else if args.optimize {
            compile_file_verified(path)?
        } else {
            compile_file_unoptimized_verified(path)?
        }
    };

    if let Some(path) = args.emit_bytecode {
        if path.to_ascii_lowercase().ends_with(".tob") {
            write_binary_artifact(program.program(), path)?;
        } else {
            write_artifact(program.program(), path)?;
        }
    }
    if let Some(path) = args.emit_jit {
        write_jit_listing(program.program(), path)?;
    }
    if args.verbose {
        eprintln!(
            "tinylang: mode={} optimize={} cache={} jit_threshold={} check={} slots={} functions={} structs={} modules={} fingerprint={}",
            args.mode,
            args.optimize,
            match cache_status {
                Some(CompileCacheStatus::Hit) => "hit",
                Some(CompileCacheStatus::Incremental) => "incremental",
                Some(CompileCacheStatus::Miss) => "miss",
                None => "off",
            },
            args.jit_threshold,
            args.check,
            program.program().slot_count(),
            program.program().functions().len(),
            program.program().structs().len(),
            program.program().modules().len(),
            program.fingerprint()
        );
    }
    if !args.check {
        let mut stdout = io::stdout();
        let jit_options = JitOptions::new().with_hot_back_edge_threshold(args.jit_threshold);
        run_verified_program_with_jit_options(
            &program,
            &args.mode,
            &mut stdout,
            args.inputs,
            jit_options,
        )?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<Args, String> {
        parse_args(values.iter().map(|value| (*value).to_string()))
    }

    #[test]
    fn ergonomic_execution_and_optimization_flags_parse() {
        let args = parse(&[
            "tinylang",
            "--vm",
            "-O0",
            "--jit-threshold",
            "3",
            "program.to",
        ])
        .unwrap();
        assert_eq!(args.mode, "vm");
        assert!(!args.optimize);
        assert!(args.cache);
        assert_eq!(args.jit_threshold, 3);
        assert_eq!(args.path.as_deref(), Some("program.to"));

        let args = parse(&["tinylang", "-j", "--no-jit-quickening", "program.to"]).unwrap();
        assert_eq!(args.mode, "jit");
        assert_eq!(args.jit_threshold, 0);

        let args = parse(&["tinylang", "--no-cache", "program.to"]).unwrap();
        assert!(!args.cache);
    }

    #[test]
    fn jit_threshold_rejects_out_of_range_values() {
        let error = parse(&["tinylang", "--jit-threshold", "65536", "program.to"]).unwrap_err();
        assert!(error.contains("0 to 65535"));
    }
}
