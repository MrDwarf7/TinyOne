use std::io::Write;

use crate::{
    JitCache, Program, Result, TinyMemory, TinyOneError, TinyRunReport, VM, compile_source,
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
    match RunMode::parse(mode)? {
        RunMode::Vm => VM::new(program, TinyMemory::new(program.slot_count), inputs).run(stdout),
        RunMode::Jit => {
            let mut cache = JitCache::new();
            cache.run_program(program, stdout, inputs)
        }
    }
}

pub fn run_program_report(
    program: &Program,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyRunReport> {
    match RunMode::parse(mode)? {
        RunMode::Vm => {
            VM::new(program, TinyMemory::new(program.slot_count), inputs).run_report(stdout)
        }
        RunMode::Jit => {
            let mut cache = JitCache::new();
            cache.run_program_report(program, stdout, inputs)
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
