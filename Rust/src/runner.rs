use std::collections::HashMap;
use std::io::Write;

use crate::{
    BytecodeVerifier, JitCache, Program, Result, TinyMemory, TinyOneError, TinyRunReport, VM,
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
    program: &Program,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyMemory> {
    run_program_with_env(program, mode, stdout, inputs, Vec::new(), HashMap::new())
}

pub fn run_program_with_env(
    program: &Program,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
    sys_args: Vec<String>,
    sys_env: HashMap<String, String>,
) -> Result<TinyMemory> {
    BytecodeVerifier::verify(program)?;
    let mode = RunMode::parse(mode)?;
    match mode {
        RunMode::Vm => {
            let mut vm = VM::new_unchecked(program, TinyMemory::new(program.slot_count), inputs);
            vm.set_sys_args(sys_args);
            vm.set_sys_env(sys_env);
            vm.run(stdout)
        }
        RunMode::Jit => {
            let mut cache = JitCache::new();
            cache.run_program_with_env_unchecked(program, stdout, inputs, sys_args, sys_env)
        }
    }
}

pub fn run_program_report(
    program: &Program,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    BytecodeVerifier::verify(program)?;
    let mode = RunMode::parse(mode)?;
    match mode {
        RunMode::Vm => {
            VM::new_unchecked(program, TinyMemory::new(program.slot_count), inputs)
                .run_report(stdout)
        }
        RunMode::Jit => {
            let mut cache = JitCache::new();
            cache.run_program_report_unchecked(program, stdout, inputs)
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
    run_program(&program, mode, stdout, inputs)
}

pub fn run_source_report(
    source: &str,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    let program = compile_source(source)?;
    run_program_report(&program, mode, stdout, inputs)
}
