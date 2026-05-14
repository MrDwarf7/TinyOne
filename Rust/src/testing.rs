//! Feature-gated testing hooks for external TinyOne harnesses.
//!
//! This module is only compiled with `--features testing-hooks`. It exposes
//! compiler/runtime inspection surfaces for tests without making them part of
//! the production API contract.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{Program, Result, RuntimeValue, TinyHeapStats, internal_testing};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestProgramInspection {
    pub fingerprint: String,
    pub slot_count: usize,
    pub main_ops: Vec<String>,
    pub function_names: Vec<String>,
    pub string_count: usize,
    pub struct_names: Vec<String>,
    pub module_names: Vec<String>,
    pub opcode_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestBackendRun {
    pub mode: &'static str,
    pub stdout: String,
    pub memory: Vec<RuntimeValue>,
    pub heap_before_shutdown: TinyHeapStats,
    pub heap_after_shutdown: TinyHeapStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestJitInspection {
    pub fingerprint: String,
    pub listing: String,
    pub chunk_count: usize,
    pub op_count: usize,
}

pub fn compile_fixture(path: impl AsRef<Path>) -> Result<Program> {
    internal_testing::compile_fixture(path)
}

pub fn compile_source_fixture(source: &str, filename: &str) -> Result<Program> {
    internal_testing::compile_source_fixture(source, filename)
}

pub fn inspect_program(program: &Program) -> TestProgramInspection {
    internal_testing::inspect_program(program).into()
}

pub fn inspect_jit(program: &Program) -> TestJitInspection {
    internal_testing::inspect_jit(program).into()
}

pub fn run_backend(
    program: &Program,
    mode: &'static str,
    inputs: Vec<String>,
) -> Result<TestBackendRun> {
    internal_testing::run_backend(program, mode, inputs).map(Into::into)
}

pub fn assert_backends_match(
    program: &Program,
    inputs: &[String],
) -> Result<(TestBackendRun, TestBackendRun)> {
    let (vm, jit) = internal_testing::assert_backends_match(program, inputs)?;
    Ok((vm.into(), jit.into()))
}

pub fn write_backend_report(
    program: &Program,
    mode: &'static str,
    inputs: Vec<String>,
    out: &mut dyn std::io::Write,
) -> Result<()> {
    internal_testing::write_backend_report(program, mode, inputs, out)
}

impl From<internal_testing::ProgramInspection> for TestProgramInspection {
    fn from(value: internal_testing::ProgramInspection) -> Self {
        Self {
            fingerprint: value.fingerprint,
            slot_count: value.slot_count,
            main_ops: value.main_ops,
            function_names: value.function_names,
            string_count: value.string_count,
            struct_names: value.struct_names,
            module_names: value.module_names,
            opcode_counts: value.opcode_counts,
        }
    }
}

impl From<internal_testing::BackendRunInspection> for TestBackendRun {
    fn from(value: internal_testing::BackendRunInspection) -> Self {
        Self {
            mode: value.mode,
            stdout: value.stdout,
            memory: value.memory,
            heap_before_shutdown: value.heap_before_shutdown,
            heap_after_shutdown: value.heap_after_shutdown,
        }
    }
}

impl From<internal_testing::JitInspection> for TestJitInspection {
    fn from(value: internal_testing::JitInspection) -> Self {
        Self {
            fingerprint: value.fingerprint,
            listing: value.listing,
            chunk_count: value.chunk_count,
            op_count: value.op_count,
        }
    }
}
