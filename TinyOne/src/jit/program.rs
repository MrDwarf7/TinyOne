use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::{
    JitChunk, JitOptions, JitStats, JitVm, Program, Result, TinyMemory, TinyOneError,
    TinyRunReport, VerifiedProgram,
};

#[derive(Debug, Clone)]
pub struct JitProgram {
    pub(crate) verified_program: VerifiedProgram,
    pub(crate) fingerprint: String,
    pub(crate) chunks: Vec<Option<JitChunk>>,
    pub(crate) options: JitOptions,
    pub(crate) stats: JitStats,
}

impl JitProgram {
    pub fn compile(program: &Program) -> crate::Result<Self> {
        let verified = VerifiedProgram::verify(program.clone())?;
        let fingerprint = verified.fingerprint().to_owned();
        Self::compile_with_fingerprint(&verified, fingerprint, JitOptions::default())
    }

    pub fn compile_with_options(program: &Program, options: JitOptions) -> crate::Result<Self> {
        let verified = VerifiedProgram::verify(program.clone())?;
        let fingerprint = verified.fingerprint().to_owned();
        Self::compile_with_fingerprint(&verified, fingerprint, options)
    }

    pub fn compile_verified(verified: &VerifiedProgram) -> Result<Self> {
        Self::compile_verified_with_options(verified, JitOptions::default())
    }

    pub fn compile_verified_with_options(
        verified: &VerifiedProgram,
        options: JitOptions,
    ) -> Result<Self> {
        Self::compile_with_fingerprint(verified, verified.fingerprint().to_owned(), options)
    }

    pub(crate) fn compile_with_fingerprint(
        verified: &VerifiedProgram,
        fingerprint: String,
        options: JitOptions,
    ) -> Result<Self> {
        let program = verified.program();
        let main = JitChunk::compile("main", program.slot_count, &program.code)?;
        let compiled_ops = main.ops.len();
        let mut chunks = Vec::with_capacity(program.functions.len() + 1);
        chunks.push(Some(main));
        chunks.resize_with(program.functions.len() + 1, || None);
        Ok(Self {
            verified_program: verified.clone(),
            fingerprint,
            chunks,
            options,
            stats: JitStats {
                compiled_chunks: 1,
                compiled_ops,
                ..JitStats::default()
            },
        })
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn stats(&self) -> JitStats {
        self.stats
    }

    pub fn listing(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        let _ = writeln!(&mut out, "; tinyone adaptive-jit {}", self.fingerprint);
        let _ = writeln!(
            &mut out,
            "; chunks={} ops={} hot_back_edges={} hot_ranges={} quickened_ops={} hot_threshold={}",
            self.stats.compiled_chunks,
            self.stats.compiled_ops,
            self.stats.hot_back_edges,
            self.stats.hot_ranges,
            self.stats.quickened_ops,
            self.options.hot_back_edge_threshold()
        );
        for (chunk_index, chunk) in self.chunks.iter().enumerate() {
            if let Some(chunk) = chunk {
                let _ = writeln!(
                    &mut out,
                    ".chunk {chunk_index} {} slots={} ops={}",
                    chunk.name,
                    chunk.slot_count,
                    chunk.ops.len()
                );
                for (pc, op) in chunk.ops.iter().enumerate() {
                    let _ = writeln!(&mut out, "  {pc:04} {}", op.listing());
                }
            } else {
                let function_index = chunk_index.saturating_sub(1);
                if let Some(function) = self
                    .verified_program
                    .program()
                    .functions
                    .get(function_index)
                {
                    let _ = writeln!(
                        &mut out,
                        ".lazy {chunk_index} {} slots={} bytecode_ops={}",
                        function.name,
                        function.slot_count,
                        function.code.len()
                    );
                }
            }
        }
        out
    }

    pub(crate) fn ensure_chunk(&mut self, chunk_index: usize) -> Result<()> {
        let slot = self
            .chunks
            .get(chunk_index)
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid JIT chunk {chunk_index}")))?;
        if slot.is_some() {
            return Ok(());
        }
        let function_index = chunk_index.checked_sub(1).ok_or_else(|| {
            TinyOneError::runtime(format!("Invalid lazy JIT chunk {chunk_index}"))
        })?;
        let function = self
            .verified_program
            .program()
            .functions
            .get(function_index)
            .ok_or_else(|| {
                TinyOneError::runtime(format!("Invalid function index {function_index}"))
            })?;
        let chunk = JitChunk::compile(function.name.clone(), function.slot_count, &function.code)?;
        self.stats.compiled_chunks += 1;
        self.stats.compiled_ops += chunk.ops.len();
        self.chunks[chunk_index] = Some(chunk);
        Ok(())
    }

    fn compile_all(&mut self) -> Result<()> {
        for chunk_index in 1..self.chunks.len() {
            self.ensure_chunk(chunk_index)?;
        }
        Ok(())
    }

    pub fn run(&mut self, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyMemory> {
        JitVm::new(self, inputs).run(stdout)
    }

    pub fn run_with_env(
        &mut self,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
        sys_args: Vec<String>,
        sys_env: HashMap<String, String>,
    ) -> Result<TinyMemory> {
        let mut vm = JitVm::new(self, inputs);
        vm.set_sys_args(sys_args);
        vm.set_sys_env(sys_env);
        vm.run(stdout)
    }

    pub fn run_report(
        &mut self,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyRunReport> {
        JitVm::new(self, inputs).run_report(stdout)
    }

    pub(crate) fn record_back_edge(&mut self, chunk_index: usize, op_pc: usize, target: usize) {
        let threshold = self.options.hot_back_edge_threshold();
        if threshold == 0 || target >= op_pc {
            return;
        }
        self.stats.hot_back_edges += 1;
        let changed = {
            let Some(chunk) = self.chunks.get_mut(chunk_index).and_then(Option::as_mut) else {
                return;
            };
            let Some(counter) = chunk.edge_counts.get_mut(op_pc) else {
                return;
            };
            *counter = counter.saturating_add(1);
            if *counter == threshold {
                chunk.promote_range(target, op_pc + 1)
            } else {
                0
            }
        };
        if changed > 0 {
            self.stats.hot_ranges += 1;
            self.stats.quickened_ops += changed;
        }
    }
}

pub fn write_jit_listing(program: &Program, path: impl AsRef<Path>) -> Result<()> {
    let mut compiled = JitProgram::compile(program)?;
    compiled.compile_all()?;
    fs::write(path, compiled.listing())
        .map_err(|error| TinyOneError::compile(format!("JIT listing write error: {error}")))
}
