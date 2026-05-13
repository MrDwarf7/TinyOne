use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::{
    HOT_BACK_EDGE_THRESHOLD, JitChunk, JitFunction, JitStats, JitVm, Program, Result, StructDef,
    TinyMemory, TinyOneError, TinyRunReport,
};

#[derive(Debug, Clone)]
pub struct JitProgram {
    pub(crate) fingerprint: String,
    pub(crate) chunks: Vec<JitChunk>,
    pub(crate) functions: Vec<JitFunction>,
    pub(crate) strings: Vec<String>,
    pub(crate) structs: Vec<StructDef>,
    pub(crate) fields: Vec<String>,
    pub(crate) stats: JitStats,
}

impl JitProgram {
    pub fn compile(program: &Program) -> Self {
        Self::compile_with_fingerprint(program, program.fingerprint())
    }

    pub(crate) fn compile_with_fingerprint(program: &Program, fingerprint: String) -> Self {
        let mut chunks = vec![JitChunk::compile("main", program.slot_count, &program.code)];
        let mut functions = Vec::with_capacity(program.functions.len());
        for function in &program.functions {
            let chunk_index = chunks.len();
            chunks.push(JitChunk::compile(
                function.name.clone(),
                function.slot_count,
                &function.code,
            ));
            functions.push(JitFunction {
                name: function.name.clone(),
                param_count: function.param_count,
                slot_count: function.slot_count,
                chunk_index,
            });
        }
        let compiled_ops = chunks.iter().map(|chunk| chunk.ops.len()).sum();
        let compiled_chunks = chunks.len();
        Self {
            fingerprint,
            chunks,
            functions,
            strings: program.strings.clone(),
            structs: program.structs.clone(),
            fields: program.fields.clone(),
            stats: JitStats {
                compiled_chunks,
                compiled_ops,
                ..JitStats::default()
            },
        }
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
        writeln!(&mut out, "; tinyone adaptive-jit {}", self.fingerprint).expect("write string");
        writeln!(
            &mut out,
            "; chunks={} ops={} hot_back_edges={} hot_ranges={} quickened_ops={}",
            self.stats.compiled_chunks,
            self.stats.compiled_ops,
            self.stats.hot_back_edges,
            self.stats.hot_ranges,
            self.stats.quickened_ops
        )
        .expect("write string");
        for (chunk_index, chunk) in self.chunks.iter().enumerate() {
            writeln!(
                &mut out,
                ".chunk {chunk_index} {} slots={} ops={}",
                chunk.name,
                chunk.slot_count,
                chunk.ops.len()
            )
            .expect("write string");
            for (pc, op) in chunk.ops.iter().enumerate() {
                writeln!(&mut out, "  {pc:04} {}", op.listing()).expect("write string");
            }
        }
        out
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
        if target >= op_pc {
            return;
        }
        self.stats.hot_back_edges += 1;
        let changed = {
            let Some(chunk) = self.chunks.get_mut(chunk_index) else {
                return;
            };
            let Some(counter) = chunk.edge_counts.get_mut(op_pc) else {
                return;
            };
            *counter = counter.saturating_add(1);
            if *counter == HOT_BACK_EDGE_THRESHOLD {
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
    let compiled = JitProgram::compile(program);
    fs::write(path, compiled.listing())
        .map_err(|error| TinyOneError::compile(format!("JIT listing write error: {error}")))
}
