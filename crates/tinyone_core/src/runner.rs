use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use crate::{
    JitOptions,
    JitProgram,
    Program,
    Result,
    TinyMemory,
    TinyOneError,
    TinyRunReport,
    VM,
    VerifiedProgram,
    compile_source_verified,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunMode {
    Vm,
    Jit,
}

impl RunMode {
    pub(crate) fn parse(mode: &str) -> Result<Self> {
        match mode {
            "vm" => Ok(Self::Vm),
            "jit" => Ok(Self::Jit),
            _ => Err(TinyOneError::runtime(format!("Unsupported mode {mode:?}"))),
        }
    }
}

/// Run `program` in `mode` (`"vm"` or `"jit"`), writing program output to `stdout`.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid, verification fails, or execution fails.
pub fn run_program(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyMemory> {
    run_program_with_jit_options(program, mode, stdout, inputs, JitOptions::default())
}

/// Run `program` with explicit JIT options.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid, verification fails, or execution fails.
pub fn run_program_with_jit_options(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    jit_options: JitOptions,
) -> Result<TinyMemory> {
    let verified = VerifiedProgram::verify_arc(program)?;
    run_verified_program_with_env_and_jit_options(
        &verified,
        mode,
        stdout,
        inputs,
        Vec::new(),
        HashMap::new(),
        jit_options,
    )
}

/// Run `program` with system args and environment.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid, verification fails, or execution fails.
pub fn run_program_with_env(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    sys_args: Vec<String>,
    sys_env: HashMap<String, String>,
) -> Result<TinyMemory> {
    run_program_with_env_and_jit_options(program, mode, stdout, inputs, sys_args, sys_env, JitOptions::default())
}

/// Run `program` with system args, environment, and JIT options.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid, verification fails, or execution fails.
pub fn run_program_with_env_and_jit_options(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    sys_args: Vec<String>,
    sys_env: HashMap<String, String>,
    jit_options: JitOptions,
) -> Result<TinyMemory> {
    let verified = VerifiedProgram::verify_arc(program)?;
    run_verified_program_with_env_and_jit_options(&verified, mode, stdout, inputs, sys_args, sys_env, jit_options)
}

/// Run an already-verified `program` in `mode`.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid or execution fails.
pub fn run_verified_program(
    verified: &VerifiedProgram,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyMemory> {
    run_verified_program_with_jit_options(verified, mode, stdout, inputs, JitOptions::default())
}

/// Run an already-verified `program` with explicit JIT options.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid or execution fails.
pub fn run_verified_program_with_jit_options(
    verified: &VerifiedProgram,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    jit_options: JitOptions,
) -> Result<TinyMemory> {
    run_verified_program_with_env_and_jit_options(
        verified,
        mode,
        stdout,
        inputs,
        Vec::new(),
        HashMap::new(),
        jit_options,
    )
}

/// Run an already-verified `program` with system args and environment.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid or execution fails.
pub fn run_verified_program_with_env(
    verified: &VerifiedProgram,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    sys_args: Vec<String>,
    sys_env: HashMap<String, String>,
) -> Result<TinyMemory> {
    run_verified_program_with_env_and_jit_options(
        verified,
        mode,
        stdout,
        inputs,
        sys_args,
        sys_env,
        JitOptions::default(),
    )
}

/// Run an already-verified `program` with system args, environment, and JIT options.
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid, the VM memory cannot be allocated, or
/// execution fails.
pub fn run_verified_program_with_env_and_jit_options(
    verified: &VerifiedProgram,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    sys_args: Vec<String>,
    sys_env: HashMap<String, String>,
    jit_options: JitOptions,
) -> Result<TinyMemory> {
    let mode = RunMode::parse(mode)?;
    match mode {
        RunMode::Vm => {
            let slot_count = verified.program().slot_count;
            let memory = TinyMemory::try_new(slot_count)?;
            let mut vm = VM::new_unchecked(verified, memory, inputs);
            vm.set_sys_args(sys_args);
            vm.set_sys_env(sys_env);
            vm.run(stdout)
        }
        RunMode::Jit => {
            let mut program = JitProgram::compile_verified_with_options(verified, jit_options)?;
            program.run_with_env(stdout, inputs, sys_args, sys_env)
        }
    }
}

/// Run `program` in `mode`, collecting a [`TinyRunReport`].
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid, verification fails, or execution fails.
pub fn run_program_report(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    let verified = VerifiedProgram::verify_arc(program)?;
    run_verified_program_report(&verified, mode, stdout, inputs)
}

/// Run an already-verified `program`, collecting a [`TinyRunReport`].
///
/// # Errors
///
/// Returns [`Err`] if `mode` is invalid, the VM memory cannot be allocated, or
/// execution fails.
pub fn run_verified_program_report(
    verified: &VerifiedProgram,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    let mode = RunMode::parse(mode)?;
    match mode {
        RunMode::Vm => {
            let slot_count = verified.program().slot_count;
            let memory = TinyMemory::try_new(slot_count)?;
            let vm = VM::new_unchecked(verified, memory, inputs);
            vm.run_report(stdout)
        }
        RunMode::Jit => {
            let mut program = JitProgram::compile_verified(verified)?;
            program.run_report(stdout, inputs)
        }
    }
}

/// Compile and run `source` in `mode`.
///
/// # Errors
///
/// Returns [`Err`] if source compilation/verification, `mode` is invalid, or
/// execution fails.
pub fn run_source(source: &str, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyMemory> {
    let program = compile_source_verified(source)?;
    run_verified_program(&program, mode, stdout, inputs)
}

/// Compile and run `source` in `mode`, collecting a [`TinyRunReport`].
///
/// # Errors
///
/// Returns [`Err`] if source compilation/verification, `mode` is invalid, or
/// execution fails.
pub fn run_source_report(
    source: &str,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    let program = compile_source_verified(source)?;
    run_verified_program_report(&program, mode, stdout, inputs)
}
