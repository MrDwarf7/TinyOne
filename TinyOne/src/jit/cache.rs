use std::collections::{BTreeMap, HashMap};
use std::io::Write;

use crate::{
    JitOp, JitProgram, Program, Result, TinyMemory, TinyRunReport, VerifiedProgram,
    compile_source_verified, content_digest,
};

pub const DEFAULT_HOT_BACK_EDGE_THRESHOLD: u16 = 8;
pub const DEFAULT_SOURCE_CACHE_MAX_ENTRIES: usize = 128;
pub const DEFAULT_SOURCE_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Controls adaptive JIT behavior. A threshold of zero disables quickening,
/// which is useful for deterministic cold-tier profiling and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitOptions {
    hot_back_edge_threshold: u16,
    execution_profile: bool,
}

impl JitOptions {
    pub const fn new() -> Self {
        Self {
            hot_back_edge_threshold: DEFAULT_HOT_BACK_EDGE_THRESHOLD,
            execution_profile: false,
        }
    }

    pub const fn with_hot_back_edge_threshold(mut self, threshold: u16) -> Self {
        self.hot_back_edge_threshold = threshold;
        self
    }

    pub const fn hot_back_edge_threshold(self) -> u16 {
        self.hot_back_edge_threshold
    }

    /// Enables per-opcode dispatch and operand-stack accounting. This is
    /// deliberately opt-in because updating the profile is not part of the
    /// normal execution fast path.
    pub const fn with_execution_profile(mut self, enabled: bool) -> Self {
        self.execution_profile = enabled;
        self
    }

    pub const fn execution_profile_enabled(self) -> bool {
        self.execution_profile
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
    /// Number of transient operand stacks allocated because no reusable
    /// stack had enough capacity.
    pub operand_stack_allocations: u64,
    /// Number of runs that borrowed a previously allocated operand stack.
    pub operand_stack_reuses: u64,
    /// Stacks currently retained for future non-reentrant executions.
    pub cached_operand_stacks: usize,
    /// Total capacity of the retained operand stacks.
    pub cached_operand_stack_capacity: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitCacheStats {
    pub programs: usize,
    pub source_programs: usize,
    pub source_bytes: usize,
    pub source_hits: u64,
    pub source_misses: u64,
    pub source_compilations: u64,
    pub source_evictions: u64,
    pub source_bypasses: u64,
    pub compiled_chunks: usize,
    pub compiled_ops: usize,
    pub hot_back_edges: u64,
    pub hot_ranges: usize,
    pub quickened_ops: usize,
    pub operand_stack_allocations: u64,
    pub operand_stack_reuses: u64,
    pub cached_operand_stacks: usize,
    pub cached_operand_stack_capacity: usize,
}

/// Operand-stack traffic attributed to one lowered JIT opcode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitOpcodeProfile {
    pub dispatches: u64,
    pub operand_stack_pushes: u64,
    pub operand_stack_pops: u64,
}

/// Optional execution attribution for a compiled JIT program.
///
/// The profile counts the actual lowered opcodes dispatched during successful
/// or failing runs. Stack traffic is the opcode's verified stack effect, so it
/// includes stack work that a fused operation intentionally eliminates.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JitExecutionProfile {
    pub dispatches: u64,
    pub operand_stack_pushes: u64,
    pub operand_stack_pops: u64,
    pub max_operand_stack_depth: usize,
    pub opcodes: BTreeMap<String, JitOpcodeProfile>,
}

impl JitExecutionProfile {
    pub(crate) fn record(&mut self, op: JitOp, stack_depth: usize) {
        let (pushes, pops) = op.operand_stack_traffic();
        self.dispatches = self.dispatches.saturating_add(1);
        self.operand_stack_pushes = self.operand_stack_pushes.saturating_add(pushes as u64);
        self.operand_stack_pops = self.operand_stack_pops.saturating_add(pops as u64);
        self.max_operand_stack_depth = self
            .max_operand_stack_depth
            .max(stack_depth.saturating_sub(pops).saturating_add(pushes));
        let entry = self
            .opcodes
            .entry(op.profile_name().to_owned())
            .or_default();
        entry.dispatches = entry.dispatches.saturating_add(1);
        entry.operand_stack_pushes = entry.operand_stack_pushes.saturating_add(pushes as u64);
        entry.operand_stack_pops = entry.operand_stack_pops.saturating_add(pops as u64);
    }
}

#[derive(Debug, Clone)]
struct SourceCacheEntry {
    source: String,
    program: VerifiedProgram,
    last_used: u64,
}

#[derive(Debug, Clone)]
pub struct JitCache {
    cache: HashMap<String, JitProgram>,
    source_cache: HashMap<[u8; 16], Vec<SourceCacheEntry>>,
    source_cache_bytes: usize,
    source_access_clock: u64,
    source_hits: u64,
    source_misses: u64,
    source_compilations: u64,
    source_evictions: u64,
    source_bypasses: u64,
    source_cache_max_entries: usize,
    source_cache_max_bytes: usize,
    options: JitOptions,
}

impl Default for JitCache {
    fn default() -> Self {
        Self {
            cache: HashMap::new(),
            source_cache: HashMap::new(),
            source_cache_bytes: 0,
            source_access_clock: 0,
            source_hits: 0,
            source_misses: 0,
            source_compilations: 0,
            source_evictions: 0,
            source_bypasses: 0,
            source_cache_max_entries: DEFAULT_SOURCE_CACHE_MAX_ENTRIES,
            source_cache_max_bytes: DEFAULT_SOURCE_CACHE_MAX_BYTES,
            options: JitOptions::default(),
        }
    }
}

impl JitCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: JitOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    pub fn options(&self) -> JitOptions {
        self.options
    }

    /// Bound the exact-source verified-program cache. Either limit may be
    /// zero to disable source caching while retaining the compiled JIT cache.
    pub fn with_source_cache_limits(mut self, max_entries: usize, max_source_bytes: usize) -> Self {
        self.source_cache_max_entries = max_entries;
        self.source_cache_max_bytes = max_source_bytes;
        while self.source_cache_len() > max_entries || self.source_cache_bytes > max_source_bytes {
            if !self.evict_lru_source() {
                break;
            }
        }
        self
    }

    pub fn source_cache_max_entries(&self) -> usize {
        self.source_cache_max_entries
    }

    pub fn source_cache_byte_limit(&self) -> usize {
        self.source_cache_max_bytes
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Number of exact source strings with a cached verified compilation.
    pub fn source_cache_len(&self) -> usize {
        self.source_cache.values().map(Vec::len).sum()
    }

    /// Exact source text bytes retained by the verified-program cache.
    pub fn source_cache_bytes(&self) -> usize {
        self.source_cache_bytes
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
        let mut stats = self
            .cache
            .values()
            .fold(JitCacheStats::default(), |mut stats, program| {
                let program_stats = program.stats();
                stats.programs += 1;
                stats.compiled_chunks += program_stats.compiled_chunks;
                stats.compiled_ops += program_stats.compiled_ops;
                stats.hot_back_edges += program_stats.hot_back_edges;
                stats.hot_ranges += program_stats.hot_ranges;
                stats.quickened_ops += program_stats.quickened_ops;
                stats.operand_stack_allocations += program_stats.operand_stack_allocations;
                stats.operand_stack_reuses += program_stats.operand_stack_reuses;
                stats.cached_operand_stacks += program_stats.cached_operand_stacks;
                stats.cached_operand_stack_capacity += program_stats.cached_operand_stack_capacity;
                stats
            });
        stats.source_programs = self.source_cache_len();
        stats.source_bytes = self.source_cache_bytes;
        stats.source_hits = self.source_hits;
        stats.source_misses = self.source_misses;
        stats.source_compilations = self.source_compilations;
        stats.source_evictions = self.source_evictions;
        stats.source_bypasses = self.source_bypasses;
        stats
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
        let program = self.compile_source_cached(source)?;
        self.run_verified_program(&program, stdout, inputs)
    }

    pub fn run_source_report(
        &mut self,
        source: &str,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyRunReport> {
        let program = self.compile_source_cached(source)?;
        self.run_verified_program_report(&program, stdout, inputs)
    }

    fn compile_source_cached(&mut self, source: &str) -> Result<VerifiedProgram> {
        let digest = content_digest(source.as_bytes());
        self.compile_source_cached_with_digest(source, digest)
    }

    fn compile_source_cached_with_digest(
        &mut self,
        source: &str,
        digest: [u8; 16],
    ) -> Result<VerifiedProgram> {
        self.source_access_clock = self.source_access_clock.saturating_add(1);
        let access = self.source_access_clock;
        let cached = self.source_cache.get_mut(&digest).and_then(|entries| {
            entries
                .iter_mut()
                .find(|entry| entry.source == source)
                .map(|entry| {
                    entry.last_used = access;
                    entry.program.clone()
                })
        });
        if let Some(program) = cached {
            self.source_hits = self.source_hits.saturating_add(1);
            return Ok(program);
        }

        self.source_misses = self.source_misses.saturating_add(1);
        let program = compile_source_verified(source)?;
        self.source_compilations = self.source_compilations.saturating_add(1);
        self.insert_source_cache(digest, source, &program, access);
        Ok(program)
    }

    fn insert_source_cache(
        &mut self,
        digest: [u8; 16],
        source: &str,
        program: &VerifiedProgram,
        last_used: u64,
    ) {
        let source_bytes = source.len();
        let max_entries = self.source_cache_max_entries;
        let max_bytes = self.source_cache_max_bytes;
        if max_entries == 0 || max_bytes == 0 || source_bytes > max_bytes {
            self.source_bypasses = self.source_bypasses.saturating_add(1);
            return;
        }

        while self.source_cache_len() >= max_entries
            || self.source_cache_bytes.saturating_add(source_bytes) > max_bytes
        {
            if !self.evict_lru_source() {
                self.source_bypasses = self.source_bypasses.saturating_add(1);
                return;
            }
        }

        self.source_cache
            .entry(digest)
            .or_default()
            .push(SourceCacheEntry {
                source: source.to_owned(),
                program: program.clone(),
                last_used,
            });
        self.source_cache_bytes += source_bytes;
    }

    fn evict_lru_source(&mut self) -> bool {
        let oldest = self
            .source_cache
            .iter()
            .flat_map(|(digest, entries)| {
                entries
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (entry.last_used, *digest, index))
            })
            .min();
        let Some((_, digest, index)) = oldest else {
            return false;
        };

        let (removed_bytes, remove_bucket) = {
            let entries = self
                .source_cache
                .get_mut(&digest)
                .expect("LRU source bucket exists");
            let entry = entries.remove(index);
            (entry.source.len(), entries.is_empty())
        };
        if remove_bucket {
            self.source_cache.remove(&digest);
        }
        self.source_cache_bytes = self.source_cache_bytes.saturating_sub(removed_bytes);
        self.source_evictions = self.source_evictions.saturating_add(1);
        true
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

    fn run_source_with_digest(cache: &mut JitCache, source: &str, digest: [u8; 16]) -> Vec<u8> {
        let program = cache
            .compile_source_cached_with_digest(source, digest)
            .expect("source compiles");
        let mut stdout = Vec::new();
        cache
            .run_verified_program(&program, &mut stdout, Vec::new())
            .expect("source runs");
        stdout
    }

    #[test]
    fn source_cache_limits_have_bounded_defaults_and_are_configurable() {
        let defaults = JitCache::new();
        assert_eq!(
            defaults.source_cache_max_entries(),
            DEFAULT_SOURCE_CACHE_MAX_ENTRIES
        );
        assert_eq!(
            defaults.source_cache_byte_limit(),
            DEFAULT_SOURCE_CACHE_MAX_BYTES
        );

        let configured = JitCache::new().with_source_cache_limits(3, 1_024);
        assert_eq!(configured.source_cache_max_entries(), 3);
        assert_eq!(configured.source_cache_byte_limit(), 1_024);
    }

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

    #[test]
    fn repeated_source_runs_reuse_verified_compilation() {
        let mut cache = JitCache::new();
        for _ in 0..2 {
            let mut stdout = Vec::new();
            cache
                .run_source(LOOP, &mut stdout, Vec::new())
                .expect("source should run");
            assert_eq!(stdout, b"4\n");
        }

        assert_eq!(cache.source_cache_len(), 1);
        assert_eq!(cache.source_cache_bytes(), LOOP.len());
        let stats = cache.stats();
        assert_eq!(stats.source_programs, 1);
        assert_eq!(stats.source_bytes, LOOP.len());
        assert_eq!(stats.source_hits, 1);
        assert_eq!(stats.source_misses, 1);
        assert_eq!(stats.source_compilations, 1);
        assert_eq!(stats.source_evictions, 0);
        assert_eq!(stats.source_bypasses, 0);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn source_cache_uses_exact_source_identity() {
        let mut cache = JitCache::new();
        for (source, expected) in [("print 1", b"1\n"), ("print 2", b"2\n")] {
            let mut stdout = Vec::new();
            cache
                .run_source(source, &mut stdout, Vec::new())
                .expect("source should run");
            assert_eq!(stdout, expected);
        }

        assert_eq!(cache.source_cache_len(), 2);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn same_digest_bucket_still_requires_exact_source_identity() {
        let mut cache = JitCache::new();
        let forced_collision = [0xA5; 16];

        assert_eq!(
            run_source_with_digest(&mut cache, "print 1", forced_collision),
            b"1\n"
        );
        assert_eq!(
            run_source_with_digest(&mut cache, "print 2", forced_collision),
            b"2\n"
        );
        assert_eq!(
            run_source_with_digest(&mut cache, "print 1", forced_collision),
            b"1\n"
        );

        let stats = cache.stats();
        assert_eq!(stats.source_programs, 2);
        assert_eq!(stats.source_hits, 1);
        assert_eq!(stats.source_misses, 2);
        assert_eq!(stats.source_compilations, 2);
    }

    #[test]
    fn entry_limit_evicts_lru_source_and_recompiles_on_next_use() {
        let mut cache = JitCache::new().with_source_cache_limits(1, 1_024);

        for (source, expected) in [
            ("print 1", &b"1\n"[..]),
            ("print 2", &b"2\n"[..]),
            ("print 1", &b"1\n"[..]),
        ] {
            let mut stdout = Vec::new();
            cache
                .run_source(source, &mut stdout, Vec::new())
                .expect("source runs");
            assert_eq!(stdout, expected);
        }

        let stats = cache.stats();
        assert_eq!(stats.source_programs, 1);
        assert_eq!(stats.source_misses, 3);
        assert_eq!(stats.source_compilations, 3);
        assert_eq!(stats.source_evictions, 2);
        assert_eq!(stats.source_bypasses, 0);
        assert_eq!(cache.len(), 2, "compiled JIT programs remain reusable");
    }

    #[test]
    fn source_hits_refresh_deterministic_lru_order() {
        let mut cache = JitCache::new().with_source_cache_limits(2, 1_024);
        for source in ["print 1", "print 2", "print 1", "print 3", "print 1"] {
            cache
                .run_source(source, &mut Vec::new(), Vec::new())
                .expect("source runs");
        }
        let before_reloading_evicted = cache.stats();
        assert_eq!(before_reloading_evicted.source_hits, 2);
        assert_eq!(before_reloading_evicted.source_misses, 3);
        assert_eq!(before_reloading_evicted.source_evictions, 1);

        cache
            .run_source("print 2", &mut Vec::new(), Vec::new())
            .expect("evicted source recompiles");
        let stats = cache.stats();
        assert_eq!(stats.source_programs, 2);
        assert_eq!(stats.source_compilations, 4);
        assert_eq!(stats.source_evictions, 2);
    }

    #[test]
    fn byte_limit_is_enforced_and_oversized_sources_are_not_retained() {
        let one_source_bytes = "print 1".len();
        let mut cache = JitCache::new().with_source_cache_limits(4, one_source_bytes);
        cache
            .run_source("print 1", &mut Vec::new(), Vec::new())
            .expect("first source runs");
        cache
            .run_source("print 2", &mut Vec::new(), Vec::new())
            .expect("second source runs");
        assert_eq!(cache.source_cache_len(), 1);
        assert_eq!(cache.source_cache_bytes(), one_source_bytes);
        assert_eq!(cache.stats().source_evictions, 1);

        let mut bypassed = JitCache::new().with_source_cache_limits(4, one_source_bytes - 1);
        for _ in 0..2 {
            bypassed
                .run_source("print 1", &mut Vec::new(), Vec::new())
                .expect("oversized source still runs");
        }
        let stats = bypassed.stats();
        assert_eq!(stats.source_programs, 0);
        assert_eq!(stats.source_bytes, 0);
        assert_eq!(stats.source_misses, 2);
        assert_eq!(stats.source_compilations, 2);
        assert_eq!(stats.source_bypasses, 2);
    }
}
