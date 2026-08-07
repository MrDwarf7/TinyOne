use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use crate::{
    JitCache, Program, Result, TinyMemory, TinyOneError, TinyRunReport, VM, VerifiedProgram,
    compile_source,
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

pub fn run_program(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyMemory> {
    run_program_with_env(program, mode, stdout, inputs, Vec::new(), HashMap::new())
}

pub fn run_program_with_env(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    sys_args: Vec<String>,
    sys_env: HashMap<String, String>,
) -> Result<TinyMemory> {
    let verified = VerifiedProgram::verify_arc(Arc::clone(&program))?;
    let mode = RunMode::parse(mode)?;
    match mode {
        RunMode::Vm => {
            let slot_count = verified.program().slot_count;
            let mut vm = VM::new_unchecked(&verified, TinyMemory::new(slot_count), inputs);
            vm.context.program_arc = Some(Arc::clone(&program));
            vm.set_sys_args(sys_args);
            vm.set_sys_env(sys_env);
            vm.run(stdout)
        }
        RunMode::Jit => {
            let mut cache = JitCache::new();
            cache.run_program_with_env_unchecked(&verified, stdout, inputs, sys_args, sys_env)
        }
    }
}

pub fn run_program_report(
    program: Arc<Program>,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    let verified = VerifiedProgram::verify_arc(Arc::clone(&program))?;
    let mode = RunMode::parse(mode)?;
    match mode {
        RunMode::Vm => {
            let slot_count = verified.program().slot_count;
            let mut vm = VM::new_unchecked(&verified, TinyMemory::new(slot_count), inputs);
            vm.context.program_arc = Some(Arc::clone(&program));
            vm.run_report(stdout)
        }
        RunMode::Jit => {
            let mut cache = JitCache::new();
            cache.run_program_report_unchecked(&verified, stdout, inputs)
        }
    }
}

pub fn run_source(
    source: &str,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyMemory> {
    let program = compile_source(source)?;
    run_program(program, mode, stdout, inputs)
}

pub fn run_source_report(
    source: &str,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    let program = compile_source(source)?;
    run_program_report(program, mode, stdout, inputs)
}
