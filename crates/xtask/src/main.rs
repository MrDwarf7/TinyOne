use std::process::ExitCode;
use std::str::FromStr;

use tinyone_xtask::{Task, plan_for, repo_root_from_cwd, usage};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let Some(task_arg) = args.next() else {
        return Err(usage());
    };

    if task_arg == "-h" || task_arg == "--help" {
        println!("{}", usage());
        return Ok(());
    }

    let task = Task::from_str(&task_arg)?;
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}\n\n{}", usage())),
        }
    }

    let repo_root = repo_root_from_cwd()?;
    plan_for(task).run(&repo_root, dry_run)
}
