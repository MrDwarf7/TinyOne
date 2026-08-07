use std::collections::HashMap;
use std::io::Write;

use crate::{
    JitProgram, Program, Result, TinyMemory, TinyRunReport, VerifiedProgram, compile_source,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitStats {
    pub compiled_chunks: usize,
    pub compiled_ops: usize,
    pub hot_back_edges: u64,
    pub hot_ranges: usize,
    pub quickened_ops: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitCacheStats {
    pub programs: usize,
    pub compiled_chunks: usize,
    pub compiled_ops: usize,
    pub hot_back_edges: u64,
    pub hot_ranges: usize,
    pub quickened_ops: usize,
}

#[derive(Debug, Default, Clone)]
pub struct JitCache {
    cache: HashMap<String, JitProgram>,
}

impl JitCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn compile(&mut self, program: &Program) -> crate::Result<&JitProgram> {
        let verified = VerifiedProgram::verify(program.clone())?;
        Ok(&*self.compile_mut(&verified)?)
    }

    fn compile_mut(&mut self, verified: &VerifiedProgram) -> Result<&mut JitProgram> {
        let program = verified.program();
        let key = program.fingerprint();
        if !self.cache.contains_key(&key) {
            let compiled = JitProgram::compile_with_fingerprint(verified, key.clone())?;
            self.cache.insert(key.clone(), compiled);
        }
        self.cache
            .get_mut(&key)
            .ok_or_else(|| crate::TinyOneError::compile("JIT cache insertion failed"))
    }

    pub fn stats(&self) -> JitCacheStats {
        self.cache
            .values()
            .fold(JitCacheStats::default(), |mut stats, program| {
                let program_stats = program.stats();
                stats.programs += 1;
                stats.compiled_chunks += program_stats.compiled_chunks;
                stats.compiled_ops += program_stats.compiled_ops;
                stats.hot_back_edges += program_stats.hot_back_edges;
                stats.hot_ranges += program_stats.hot_ranges;
                stats.quickened_ops += program_stats.quickened_ops;
                stats
            })
    }

    pub fn run_program(
        &mut self,
        program: &Program,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyMemory> {
        let verified = VerifiedProgram::verify(program.clone())?;
        self.run_program_unchecked(&verified, stdout, inputs)
    }

    pub fn run_program_with_env(
        &mut self,
        program: &Program,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
        sys_args: Vec<String>,
        sys_env: HashMap<String, String>,
    ) -> Result<TinyMemory> {
        let verified = VerifiedProgram::verify(program.clone())?;
        self.run_program_with_env_unchecked(&verified, stdout, inputs, sys_args, sys_env)
    }

    pub fn run_program_report(
        &mut self,
        program: &Program,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyRunReport> {
        let verified = VerifiedProgram::verify(program.clone())?;
        self.run_program_report_unchecked(&verified, stdout, inputs)
    }

    /// Run without re-verifying from a verification token.
    pub(crate) fn run_program_unchecked(
        &mut self,
        verified: &VerifiedProgram,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyMemory> {
        let compiled = self.compile_mut(verified)?;
        compiled.run(stdout, inputs)
    }

    pub(crate) fn run_program_with_env_unchecked(
        &mut self,
        verified: &VerifiedProgram,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
        sys_args: Vec<String>,
        sys_env: HashMap<String, String>,
    ) -> Result<TinyMemory> {
        let compiled = self.compile_mut(verified)?;
        compiled.run_with_env(stdout, inputs, sys_args, sys_env)
    }

    pub(crate) fn run_program_report_unchecked(
        &mut self,
        verified: &VerifiedProgram,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyRunReport> {
        let compiled = self.compile_mut(verified)?;
        compiled.run_report(stdout, inputs)
    }

    pub fn run_source(
        &mut self,
        source: &str,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyMemory> {
        let program = compile_source(source)?;
        self.run_program(&program, stdout, inputs)
    }

    pub fn run_source_report(
        &mut self,
        source: &str,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyRunReport> {
        let program = compile_source(source)?;
        self.run_program_report(&program, stdout, inputs)
    }
}
