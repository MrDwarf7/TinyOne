use std::env;
use std::io::{self, Read};
use std::sync::Arc;

use tinyone::{TinyOneError, compile_file, load_artifact, run_program, write_artifact, write_jit_listing};

#[derive(Debug)]
struct Args {
    path:          Option<String>,
    mode:          String,
    check:         bool,
    emit_bytecode: Option<String>,
    emit_jit:      Option<String>,
    run_bytecode:  Option<String>,
    inputs:        Vec<String>,
    stdin:         bool,
    verbose:       bool,
    help:          bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            path:          None,
            mode:          "jit".to_string(),
            check:         false,
            emit_bytecode: None,
            emit_jit:      None,
            run_bytecode:  None,
            inputs:        Vec::new(),
            stdin:         false,
            verbose:       false,
            help:          false,
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
                args.inputs.push(iter.next().ok_or("--input requires a value")?);
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
    println!("  --mode {{jit,vm}}       Execution mode (default: jit)");
    println!("  --check                Compile only, do not run");
    println!("  --emit-bytecode PATH   Write a bytecode artifact to PATH");
    println!("  --emit-jit PATH        Write a JIT listing to PATH");
    println!("  --run-bytecode PATH    Run a compiled bytecode artifact");
    println!("  --input VALUE          Supply a program input value (repeatable)");
    println!("  --stdin                Read input values from stdin");
    println!("  --verbose              Print program metadata before running");
    println!("  -h, --help             Show this help message");
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

    let program = if let Some(path) = args.run_bytecode {
        Arc::new(load_artifact(path)?)
    } else {
        let Some(path) = args.path else {
            return Err(TinyOneError::Compile("File error: a source path is required".to_string()));
        };
        compile_file(path)?
    };

    if let Some(path) = args.emit_bytecode {
        write_artifact(&*program, path)?;
    }
    if let Some(path) = args.emit_jit {
        write_jit_listing(&*program, path)?;
    }
    if args.verbose {
        eprintln!(
            "tinylang: mode={} check={} slots={} functions={} structs={} modules={} fingerprint={}",
            args.mode,
            args.check,
            program.slot_count,
            program.functions.len(),
            program.structs.len(),
            program.modules.len(),
            program.fingerprint()
        );
    }
    if !args.check {
        let mut stdout = io::stdout();
        run_program(program, &args.mode, &mut stdout, args.inputs)?;
    }
    Ok(0)
}
