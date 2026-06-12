#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

use crate::{
    JitProgram,
    Program,
    Result,
    RuntimeValue,
    TinyHeapStats,
    TinyOneError,
    compile_file,
    compile_source_with_filename,
    run_program_report,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgramInspection {
    pub(crate) fingerprint:    String,
    pub(crate) slot_count:     usize,
    pub(crate) main_ops:       Vec<String>,
    pub(crate) function_names: Vec<String>,
    pub(crate) string_count:   usize,
    pub(crate) struct_names:   Vec<String>,
    pub(crate) module_names:   Vec<String>,
    pub(crate) opcode_counts:  BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BackendRunInspection {
    pub(crate) mode:                 &'static str,
    pub(crate) stdout:               String,
    pub(crate) memory:               Vec<RuntimeValue>,
    pub(crate) heap_before_shutdown: TinyHeapStats,
    pub(crate) heap_after_shutdown:  TinyHeapStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JitInspection {
    pub(crate) fingerprint: String,
    pub(crate) listing:     String,
    pub(crate) chunk_count: usize,
    pub(crate) op_count:    usize,
}

pub(crate) fn compile_fixture(path: impl AsRef<std::path::Path>) -> Result<Arc<Program>> {
    compile_file(path)
}

pub(crate) fn compile_source_fixture(source: &str, filename: &str) -> Result<Arc<Program>> {
    compile_source_with_filename(source, filename)
}

pub(crate) fn inspect_program(program: &Program) -> ProgramInspection {
    let mut opcode_counts = BTreeMap::new();
    for instr in &program.code {
        *opcode_counts.entry(instr.op.name().to_string()).or_insert(0) += 1;
    }

    ProgramInspection {
        fingerprint: program.fingerprint(),
        slot_count: program.slot_count,
        main_ops: program.code.iter().map(|instr| instr.op.name().to_string()).collect(),
        function_names: program.functions.iter().map(|function| function.name.clone()).collect(),
        string_count: program.strings.len(),
        struct_names: program
            .structs
            .iter()
            .map(|struct_def| struct_def.name.clone())
            .collect(),
        module_names: program.modules.iter().map(|module| module.name.clone()).collect(),
        opcode_counts,
    }
}

pub(crate) fn inspect_jit(program: &Program) -> JitInspection {
    match JitProgram::compile(program) {
        Ok(compiled) => {
            JitInspection {
                fingerprint: compiled.fingerprint().to_string(),
                listing:     compiled.listing(),
                chunk_count: compiled.chunks.len(),
                op_count:    compiled.chunks.iter().map(|chunk| chunk.ops.len()).sum::<usize>(),
            }
        }
        Err(error) => {
            JitInspection {
                fingerprint: program.fingerprint(),
                listing:     format!("; tinyone adaptive-jit unavailable: {error}"),
                chunk_count: 0,
                op_count:    0,
            }
        }
    }
}

pub(crate) fn run_backend(
    program: Arc<Program>,
    mode: &'static str,
    inputs: Vec<String>,
) -> Result<BackendRunInspection> {
    let mut stdout = Vec::new();
    let report = run_program_report(program, mode, &mut stdout, inputs)?;
    Ok(BackendRunInspection {
        mode,
        stdout: String::from_utf8(stdout)
            .map_err(|error| TinyOneError::runtime(format!("Non-UTF-8 stdout: {error}")))?,
        memory: report.memory.snapshot(),
        heap_before_shutdown: report.heap_before_shutdown,
        heap_after_shutdown: report.heap_after_shutdown,
    })
}

pub(crate) fn assert_backends_match(
    program: Arc<Program>,
    inputs: &[String],
) -> Result<(BackendRunInspection, BackendRunInspection)> {
    let vm = run_backend(Arc::clone(&program), "vm", inputs.to_vec())?;
    let jit = run_backend(Arc::clone(&program), "jit", inputs.to_vec())?;
    if vm.stdout != jit.stdout || vm.memory != jit.memory {
        return Err(TinyOneError::runtime(format!(
            "Backend mismatch: vm stdout {:?}, jit stdout {:?}, vm memory {:?}, jit memory {:?}",
            vm.stdout, jit.stdout, vm.memory, jit.memory
        )));
    }
    Ok((vm, jit))
}

pub(crate) fn write_backend_report(
    program: Arc<Program>,
    mode: &'static str,
    inputs: Vec<String>,
    out: &mut dyn Write,
) -> Result<()> {
    let run = run_backend(program, mode, inputs)?;
    writeln!(out, "mode={}", run.mode)
        .map_err(|error| TinyOneError::runtime(format!("Report write error: {error}")))?;
    writeln!(out, "stdout={:?}", run.stdout)
        .map_err(|error| TinyOneError::runtime(format!("Report write error: {error}")))?;
    writeln!(out, "memory={:?}", run.memory)
        .map_err(|error| TinyOneError::runtime(format!("Report write error: {error}")))?;
    writeln!(out, "heap_before_shutdown={:?}", run.heap_before_shutdown)
        .map_err(|error| TinyOneError::runtime(format!("Report write error: {error}")))?;
    writeln!(out, "heap_after_shutdown={:?}", run.heap_after_shutdown)
        .map_err(|error| TinyOneError::runtime(format!("Report write error: {error}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_inspection_reports_private_jit_shape() {
        let program = compile_source_fixture(
            r#"
            let i = 0
            while i < 3 {
              i = i + 1
            }
            print i
            "#,
            "internal-hook-smoke.to",
        )
        .expect("compile fixture");

        let program_inspection = inspect_program(&*program);
        assert!(program_inspection.main_ops.iter().any(|op| op == "JUMP"));

        let jit_inspection = inspect_jit(&*program);
        assert_eq!(program.fingerprint(), jit_inspection.fingerprint);
        assert!(jit_inspection.chunk_count >= 1);
        assert!(jit_inspection.op_count > 0);
        assert!(jit_inspection.listing.contains("tinyone adaptive-jit"));
    }
}
