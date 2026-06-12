use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Task {
    Check,
    Test,
    TestHooks,
    FmtCheck,
    Clippy,
    BenchSmoke,
    ToolsTest,
    ReleaseGate,
}

impl Task {
    pub const ALL: &'static [Task] = &[
        Task::Check,
        Task::Test,
        Task::TestHooks,
        Task::FmtCheck,
        Task::Clippy,
        Task::BenchSmoke,
        Task::ToolsTest,
        Task::ReleaseGate,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Task::Check => "check",
            Task::Test => "test",
            Task::TestHooks => "test-hooks",
            Task::FmtCheck => "fmt-check",
            Task::Clippy => "clippy",
            Task::BenchSmoke => "bench-smoke",
            Task::ToolsTest => "tools-test",
            Task::ReleaseGate => "release-gate",
        }
    }
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Task {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "check" => Ok(Task::Check),
            "test" => Ok(Task::Test),
            "test-hooks" => Ok(Task::TestHooks),
            "fmt-check" => Ok(Task::FmtCheck),
            "clippy" => Ok(Task::Clippy),
            "bench-smoke" => Ok(Task::BenchSmoke),
            "tools-test" => Ok(Task::ToolsTest),
            "release-gate" => Ok(Task::ReleaseGate),
            other => Err(format!("unknown task: {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    program: &'static str,
    args:    Vec<&'static str>,
}

impl CommandSpec {
    fn new(program: &'static str, args: impl Into<Vec<&'static str>>) -> Self {
        Self {
            program,
            args: args.into(),
        }
    }

    pub fn render(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(shell_word(self.program));
        parts.extend(self.args.iter().map(|arg| shell_word(arg)));
        parts.join(" ")
    }

    fn run(&self, repo_root: &Path) -> Result<ExitStatus, String> {
        Command::new(self.program)
            .args(&self.args)
            .current_dir(repo_root)
            .status()
            .map_err(|err| format!("failed to start `{}`: {err}", self.render()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Step {
    pub label:    &'static str,
    pub commands: Vec<CommandSpec>,
}

impl Step {
    fn new(label: &'static str, commands: impl Into<Vec<CommandSpec>>) -> Self {
        Self {
            label,
            commands: commands.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    pub task:  Task,
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn render_commands(&self) -> Vec<String> {
        self.steps
            .iter()
            .flat_map(|step| step.commands.iter().map(CommandSpec::render))
            .collect()
    }

    pub fn run(&self, repo_root: &Path, dry_run: bool) -> Result<(), String> {
        for step in &self.steps {
            println!("==> {}", step.label);
            for command in &step.commands {
                println!("    {}", command.render());
                if dry_run {
                    continue;
                }

                let status = command.run(repo_root)?;
                if !status.success() {
                    return Err(format!("`{}` failed with status {}", command.render(), status));
                }
            }
        }
        Ok(())
    }
}

pub fn plan_for(task: Task) -> Plan {
    let steps = match task {
        Task::Check => vec![check_step()],
        Task::Test => vec![test_step()],
        Task::TestHooks => vec![test_hooks_step()],
        Task::FmtCheck => vec![fmt_check_step()],
        Task::Clippy => vec![clippy_step()],
        Task::BenchSmoke => vec![bench_smoke_step()],
        Task::ToolsTest => vec![tools_test_step()],
        Task::ReleaseGate => {
            vec![
                check_step(),
                test_step(),
                test_hooks_step(),
                fmt_check_step(),
                clippy_step(),
                bench_smoke_step(),
                tools_test_step(),
            ]
        }
    };

    Plan { task, steps }
}

pub fn repo_root_from_cwd() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|err| format!("cannot read cwd: {err}"))?;
    if cwd.join("crates/tinyone_core/Cargo.toml").is_file()
        && cwd.join("crates/tinyone_ralloc/Cargo.toml").is_file()
        && cwd.join("tools/test_zip.py").is_file()
    {
        return Ok(cwd);
    }

    Err(format!(
        "run xtask from the TinyOne repo root; missing crates/tinyone_core/Cargo.toml, crates/tinyone_ralloc/Cargo.toml, or tools/test_zip.py under {}",
        cwd.display()
    ))
}

pub fn usage() -> String {
    let tasks = Task::ALL.iter().map(|task| task.as_str()).collect::<Vec<_>>().join("|");
    format!(
        "usage: cargo run --manifest-path crates/xtask/Cargo.toml -- <{tasks}> [--dry-run]\n\
         \n\
         CI gate: cargo run --manifest-path crates/xtask/Cargo.toml -- release-gate"
    )
}

fn check_step() -> Step {
    Step::new(
        "check",
        [
            cargo(&["check", "--manifest-path", "crates/xtask/Cargo.toml", "--all-targets"]),
            cargo(&[
                "check",
                "--manifest-path",
                "crates/tinyone_core/Cargo.toml",
                "--all-targets",
            ]),
            cargo(&[
                "check",
                "--manifest-path",
                "crates/tinyone_ralloc/Cargo.toml",
                "--workspace",
                "--all-targets",
            ]),
        ],
    )
}

fn test_step() -> Step {
    Step::new(
        "test",
        [
            cargo(&["test", "--manifest-path", "crates/xtask/Cargo.toml"]),
            cargo(&["test", "--manifest-path", "crates/tinyone_core/Cargo.toml"]),
            cargo(&[
                "test",
                "--manifest-path",
                "crates/tinyone_ralloc/Cargo.toml",
                "--workspace",
            ]),
        ],
    )
}

fn test_hooks_step() -> Step {
    Step::new(
        "test-hooks",
        [cargo(&[
            "test",
            "--manifest-path",
            "crates/tinyone_core/Cargo.toml",
            "--features",
            "testing-hooks",
        ])],
    )
}

fn fmt_check_step() -> Step {
    Step::new(
        "fmt-check",
        [
            cargo(&["fmt", "--manifest-path", "crates/xtask/Cargo.toml", "--all", "--check"]),
            cargo(&[
                "fmt",
                "--manifest-path",
                "crates/tinyone_core/Cargo.toml",
                "--all",
                "--check",
            ]),
            cargo(&[
                "fmt",
                "--manifest-path",
                "crates/tinyone_ralloc/Cargo.toml",
                "--all",
                "--check",
            ]),
        ],
    )
}

fn clippy_step() -> Step {
    Step::new(
        "clippy",
        [
            cargo(&[
                "clippy",
                "--manifest-path",
                "crates/xtask/Cargo.toml",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]),
            cargo(&[
                "clippy",
                "--manifest-path",
                "crates/tinyone_core/Cargo.toml",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]),
            cargo(&[
                "clippy",
                "--manifest-path",
                "crates/tinyone_ralloc/Cargo.toml",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]),
        ],
    )
}

fn bench_smoke_step() -> Step {
    Step::new(
        "bench-smoke",
        [cargo(&[
            "run",
            "--release",
            "--manifest-path",
            "crates/tinyone_core/Cargo.toml",
            "--bin",
            "tinylang_bench",
            "--",
            "--quick",
            "--repeats",
            "1",
            "--filter",
            "runtime.vm_straightline",
        ])],
    )
}

fn tools_test_step() -> Step {
    Step::new(
        "tools-test",
        [
            python(&[
                "-m",
                "unittest",
                "tools.test_abi_manifest",
                "tools.test_hash",
                "tools.test_loc",
                "tools.test_zip",
            ]),
            python(&["tools/abi_manifest.py", "check"]),
            python(&["tools/hash.py", "tools/hash.py", "--format", "plain"]),
            python(&["tools/loc.py", "--json"]),
        ],
    )
}

fn cargo(args: &[&'static str]) -> CommandSpec {
    CommandSpec::new("cargo", args.to_vec())
}

fn python(args: &[&'static str]) -> CommandSpec {
    CommandSpec::new("python3", args.to_vec())
}

fn shell_word(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/' | b'-' | b'_'))
    {
        return value.to_owned();
    }

    format!("'{}'", value.replace('\'', r"'\''"))
}
