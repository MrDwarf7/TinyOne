use std::collections::HashMap;
use std::io::Write;

use crate::{
    JitProgram, Program, Result, TinyMemory, TinyRunReport, VerifiedProgram,
    compile_source_verified,
};

pub const DEFAULT_HOT_BACK_EDGE_THRESHOLD: u16 = 8;

/// Controls adaptive JIT behavior. A threshold of zero disables quickening,
/// which is useful for deterministic cold-tier profiling and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitOptions {
    hot_back_edge_threshold: u16,
}

impl JitOptions {
    pub const fn new() -> Self {
        Self {
            hot_back_edge_threshold: DEFAULT_HOT_BACK_EDGE_THRESHOLD,
        }
    }

    pub const fn with_hot_back_edge_threshold(mut self, threshold: u16) -> Self {
        self.hot_back_edge_threshold = threshold;
        self
    }

    pub const fn hot_back_edge_threshold(self) -> u16 {
        self.hot_back_edge_threshold
    }

    pub const fn quickening_enabled(self) -> bool {
        self.hot_back_edge_threshold != 0
    }
}

impl Default for JitOptions {
    fn default() -> Self {
        Self::new()
    }
}

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
    options: JitOptions,
}

impl JitCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: JitOptions) -> Self {
        Self {
            cache: HashMap::new(),
            options,
        }
    }

    pub fn options(&self) -> JitOptions {
        self.options
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn compile(&mut self, program: &Program) -> crate::Result<&JitProgram> {
        let verified = VerifiedProgram::verify(program.clone())?;
        self.compile_verified(&verified)
    }

    pub fn compile_verified(&mut self, verified: &VerifiedProgram) -> Result<&JitProgram> {
        Ok(&*self.compile_mut(verified)?)
    }

    fn compile_mut(&mut self, verified: &VerifiedProgram) -> Result<&mut JitProgram> {
        let key = verified.fingerprint().to_owned();
        if !self.cache.contains_key(&key) {
            let compiled =
                JitProgram::compile_with_fingerprint(verified, key.clone(), self.options)?;
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

    pub fn run_verified_program(
        &mut self,
        verified: &VerifiedProgram,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyMemory> {
        self.run_program_unchecked(verified, stdout, inputs)
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

    pub fn run_verified_program_with_env(
        &mut self,
        verified: &VerifiedProgram,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
        sys_args: Vec<String>,
        sys_env: HashMap<String, String>,
    ) -> Result<TinyMemory> {
        self.run_program_with_env_unchecked(verified, stdout, inputs, sys_args, sys_env)
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

    pub fn run_verified_program_report(
        &mut self,
        verified: &VerifiedProgram,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyRunReport> {
        self.run_program_report_unchecked(verified, stdout, inputs)
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
        let program = compile_source_verified(source)?;
        self.run_verified_program(&program, stdout, inputs)
    }

    pub fn run_source_report(
        &mut self,
        source: &str,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyRunReport> {
        let program = compile_source_verified(source)?;
        self.run_verified_program_report(&program, stdout, inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOP: &str = r#"
        let i = 0
        while i < 4 {
          i = i + 1
        }
        print i
    "#;

    #[test]
    fn configurable_threshold_quickens_at_the_requested_back_edge() {
        let program = compile_source_verified(LOOP).unwrap();
        let mut cache = JitCache::with_options(JitOptions::new().with_hot_back_edge_threshold(1));
        let mut stdout = Vec::new();
        cache
            .run_verified_program(&program, &mut stdout, Vec::new())
            .unwrap();

        let stats = cache.stats();
        assert!(stats.hot_back_edges >= 1);
        assert!(stats.hot_ranges >= 1);
        assert!(stats.quickened_ops >= 1);
    }

    #[test]
    fn zero_threshold_keeps_the_cold_tier_stable() {
        let program = compile_source_verified(LOOP).unwrap();
        let mut cache = JitCache::with_options(JitOptions::new().with_hot_back_edge_threshold(0));
        cache
            .run_verified_program(&program, &mut Vec::new(), Vec::new())
            .unwrap();

        let stats = cache.stats();
        assert_eq!(stats.hot_back_edges, 0);
        assert_eq!(stats.hot_ranges, 0);
        assert_eq!(stats.quickened_ops, 0);
    }
}
