use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tinyone::testing;

#[derive(Debug)]
struct Fixture {
    path:            PathBuf,
    expected_stdout: Option<String>,
    inputs:          Vec<String>,
    expected_error:  Option<String>,
}

#[derive(Debug, Default)]
struct SuiteReport {
    passed: usize,
    failed: usize,
}

impl SuiteReport {
    fn record(&mut self, result: FixtureResult) {
        let status = if result.passed { "PASS" } else { "FAIL" };
        println!("  [{status}] {:<58} {}", result.path, result.detail);
        if result.passed {
            self.passed += 1;
        } else {
            self.failed += 1;
            if let Some(error) = result.error {
                println!("         {error}");
            }
        }
    }

    fn is_clean(&self) -> bool {
        self.failed == 0
    }

    fn total(&self) -> usize {
        self.passed + self.failed
    }

    fn pass_percentage(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            100.0
        } else {
            (self.passed as f64 / total as f64) * 100.0
        }
    }
}

#[derive(Debug)]
struct FixtureResult {
    path:   String,
    passed: bool,
    detail: String,
    error:  Option<String>,
}

impl FixtureResult {
    fn pass(path: &Path, detail: impl Into<String>) -> Self {
        Self {
            path:   display_path(path),
            passed: true,
            detail: detail.into(),
            error:  None,
        }
    }

    fn fail(path: &Path, detail: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            path:   display_path(path),
            passed: false,
            detail: detail.into(),
            error:  Some(error.into()),
        }
    }
}

fn main() -> ExitCode {
    println!("TinyOne feature-gated language suite");
    println!("feature: testing-hooks");
    println!();

    let mut report = SuiteReport::default();
    run_section(
        "Language pass fixtures: compile, expected stdout, and VM/JIT parity",
        "tests/language/pass",
        check_pass_fixture,
        &mut report,
    );
    run_section(
        "Programs pass fixtures: compile and VM/JIT parity",
        "tests/programs/pass",
        check_pass_fixture,
        &mut report,
    );
    run_section(
        "Programs module fixtures: import/export compatibility",
        "tests/programs/modules",
        check_pass_fixture,
        &mut report,
    );
    run_section(
        "Language compile-fail fixtures: expected diagnostics",
        "tests/language/fail_compile",
        check_compile_fail_fixture,
        &mut report,
    );
    run_section(
        "Programs compile-fail fixtures: expected compile failure",
        "tests/programs/fail_compile",
        check_compile_fail_fixture,
        &mut report,
    );
    run_section(
        "Language runtime-fail fixtures: VM/JIT expected runtime diagnostics",
        "tests/language/fail_runtime",
        check_runtime_fail_fixture,
        &mut report,
    );
    run_section(
        "Programs runtime-fail fixtures: expected VM/JIT runtime failure",
        "tests/programs/fail_runtime",
        check_runtime_fail_fixture,
        &mut report,
    );

    println!();
    println!(
        "TinyOne language suite summary: {}/{} files passed ({:.2}%), {} failed",
        report.passed,
        report.total(),
        report.pass_percentage(),
        report.failed
    );

    if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run_section(title: &str, root: &str, check: fn(&Fixture) -> FixtureResult, report: &mut SuiteReport) {
    println!("{title}");
    for fixture in fixtures(root) {
        report.record(check(&fixture));
    }
    println!();
}

fn check_pass_fixture(fixture: &Fixture) -> FixtureResult {
    let program = match testing::compile_fixture(&fixture.path) {
        Ok(program) => program,
        Err(error) => {
            return FixtureResult::fail(&fixture.path, "compile", format!("compile error: {error}"));
        }
    };

    let inspection = testing::inspect_program(&*program);
    if inspection.fingerprint.is_empty() {
        return FixtureResult::fail(&fixture.path, "inspect bytecode", "program fingerprint was empty");
    }

    let jit_inspection = testing::inspect_jit(&*program);
    if inspection.fingerprint != jit_inspection.fingerprint {
        return FixtureResult::fail(
            &fixture.path,
            "inspect jit",
            format!(
                "bytecode fingerprint {} did not match JIT fingerprint {}",
                inspection.fingerprint, jit_inspection.fingerprint
            ),
        );
    }
    if jit_inspection.op_count == 0 {
        return FixtureResult::fail(&fixture.path, "inspect jit", "JIT op count was zero");
    }

    let (vm, jit) = match testing::assert_backends_match(Arc::clone(&program), &fixture.inputs) {
        Ok(result) => result,
        Err(error) => {
            return FixtureResult::fail(&fixture.path, "vm/jit parity", format!("backend mismatch: {error}"));
        }
    };

    if let Some(expected_stdout) = &fixture.expected_stdout {
        if vm.stdout != *expected_stdout {
            return FixtureResult::fail(
                &fixture.path,
                "vm stdout",
                format!("expected {expected_stdout:?}, got {:?}", vm.stdout),
            );
        }
        if jit.stdout != *expected_stdout {
            return FixtureResult::fail(
                &fixture.path,
                "jit stdout",
                format!("expected {expected_stdout:?}, got {:?}", jit.stdout),
            );
        }
    }

    FixtureResult::pass(
        &fixture.path,
        format!(
            "stdout {:?}, inputs={}, jit_ops={}",
            compact_stdout(&vm.stdout),
            fixture.inputs.len(),
            jit_inspection.op_count
        ),
    )
}

fn check_compile_fail_fixture(fixture: &Fixture) -> FixtureResult {
    let error = match compile_expected_failure_subject(&fixture.path) {
        Ok(_) => {
            return FixtureResult::fail(&fixture.path, "compile diagnostic", "fixture compiled successfully");
        }
        Err(error) => error.to_string(),
    };

    if let Some(expected) = fixture.expected_error.as_deref() {
        if !error.contains(expected) {
            return FixtureResult::fail(
                &fixture.path,
                "compile diagnostic",
                format!("expected error containing {expected:?}, got {error:?}"),
            );
        }
        return FixtureResult::pass(&fixture.path, format!("matched {expected:?}"));
    }

    FixtureResult::pass(&fixture.path, "failed during compile as expected")
}

fn check_runtime_fail_fixture(fixture: &Fixture) -> FixtureResult {
    let program = match testing::compile_fixture(&fixture.path) {
        Ok(program) => program,
        Err(error) => {
            return FixtureResult::fail(&fixture.path, "compile", format!("compile error: {error}"));
        }
    };

    for mode in ["vm", "jit"] {
        let error = match testing::run_backend(Arc::clone(&program), mode, fixture.inputs.clone()) {
            Ok(run) => {
                return FixtureResult::fail(
                    &fixture.path,
                    format!("{mode} runtime diagnostic"),
                    format!("fixture ran successfully with stdout {:?}", run.stdout),
                );
            }
            Err(error) => error.to_string(),
        };
        if let Some(expected) = fixture.expected_error.as_deref() {
            if !error.contains(expected) {
                return FixtureResult::fail(
                    &fixture.path,
                    format!("{mode} runtime diagnostic"),
                    format!("expected error containing {expected:?}, got {error:?}"),
                );
            }
        }
    }

    match fixture.expected_error.as_deref() {
        Some(expected) => {
            FixtureResult::pass(&fixture.path, format!("vm+jit matched {expected:?}, inputs={}", fixture.inputs.len()))
        }
        None => {
            FixtureResult::pass(
                &fixture.path,
                format!("vm+jit failed at runtime as expected, inputs={}", fixture.inputs.len()),
            )
        }
    }
}

fn fixtures(root: impl AsRef<Path>) -> Vec<Fixture> {
    let root = root.as_ref();
    let mut paths = Vec::new();
    collect_to_files(root, &mut paths);
    paths.sort();
    assert!(!paths.is_empty(), "{} should contain .to fixtures", root.display());
    paths.into_iter().map(parse_fixture).collect()
}

fn collect_to_files(root: &Path, paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root)
        .unwrap_or_else(|error| panic!("failed to read fixture directory {}: {error}", root.display()))
    {
        let entry = entry.expect("fixture directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_to_files(&path, paths);
        } else if path.extension().is_some_and(|ext| ext == "to") {
            paths.push(path);
        }
    }
}

fn compile_expected_failure_subject(path: &Path) -> tinyone::Result<Arc<tinyone::Program>> {
    if path.ends_with("tests/programs/fail_compile/009_module_top_level_executable_code.to") {
        let importer = temporary_importer_for(path);
        let result = testing::compile_fixture(&importer);
        let _ = fs::remove_file(importer);
        return result;
    }
    testing::compile_fixture(path)
}

fn temporary_importer_for(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let importer =
        std::env::temp_dir().join(format!("tinyone-language-suite-import-{}-{stamp}.to", std::process::id()));
    let target = path
        .canonicalize()
        .unwrap_or_else(|error| panic!("failed to canonicalize {}: {error}", path.display()));
    fs::write(&importer, format!("import {:?} as bad\n", target.display().to_string()))
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", importer.display()));
    importer
}

fn parse_fixture(path: PathBuf) -> Fixture {
    let source = fs::read_to_string(&path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let mut expected_stdout = None;
    let mut stdout_lines = String::new();
    let mut inputs = Vec::new();
    let mut expected_error = None;
    let mut in_stdout = false;

    for line in source.lines() {
        let Some(comment) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let directive = comment.trim_start();
        match directive {
            "expect_stdout:" => {
                in_stdout = true;
                continue;
            }
            "end_expect_stdout" => {
                in_stdout = false;
                continue;
            }
            _ => {}
        }

        if in_stdout {
            stdout_lines.push_str(directive);
            stdout_lines.push('\n');
            expected_stdout = Some(stdout_lines.clone());
        } else if let Some(raw) = directive.strip_prefix("input:") {
            inputs.push(raw.trim_start().to_string());
        } else if let Some(raw) = directive.strip_prefix("expect_error:") {
            expected_error = Some(raw.trim_start().to_string());
        }
    }

    apply_legacy_program_inputs(&path, &mut inputs);

    Fixture {
        path,
        expected_stdout,
        inputs,
        expected_error,
    }
}

fn display_path(path: &Path) -> String {
    path.strip_prefix("tests").unwrap_or(path).display().to_string()
}

fn compact_stdout(stdout: &str) -> String {
    stdout.trim_end_matches('\n').replace('\n', "\\n")
}

fn apply_legacy_program_inputs(path: &Path, inputs: &mut Vec<String>) {
    if !inputs.is_empty() {
        return;
    }
    if path.ends_with("tests/programs/pass/015_input_builtins.to") {
        inputs.extend(["12", "34", "hello"].into_iter().map(String::from));
    } else if path.ends_with("tests/programs/fail_runtime/009_read_int_requires_numeric.to") {
        inputs.push("abc".to_string());
    }
}
