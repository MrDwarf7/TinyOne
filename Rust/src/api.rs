use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    BytecodeVerifier, Compiler, CompilerSharedState, Lexer, PeepholeOptimizer, Program, Result,
    TinyOneError, resolve_import,
};

pub fn compile_source(source: &str) -> Result<Arc<Program>> {
    compile_source_with_filename(source, "<source>")
}

pub fn lex_source(source: &str) -> Result<usize> {
    Ok(Lexer::new(source, "<source>").tokenize()?.len())
}

pub fn compile_source_unoptimized(source: &str) -> Result<Arc<Program>> {
    compile_source_unoptimized_with_filename(source, "<source>")
}

pub fn compile_source_unoptimized_with_filename(source: &str, filename: &str) -> Result<Arc<Program>> {
    let shared = Rc::new(RefCell::new(CompilerSharedState::default()));
    let mut compiler = Compiler::new(source, filename, None, false, "", shared)?;
    let program = compiler.compile()?;
    Ok(Arc::new(program))
}

pub fn optimize_program(program: Arc<Program>) -> Arc<Program> {
    Arc::new(PeepholeOptimizer::optimize(
        Arc::try_unwrap(program).unwrap_or_else(|arc| (*arc).clone()),
    ))
}

pub fn compile_source_with_filename(source: &str, filename: &str) -> Result<Arc<Program>> {
    let program = compile_source_unoptimized_with_filename(source, filename)?;
    let program = PeepholeOptimizer::optimize(
        Arc::try_unwrap(program).unwrap_or_else(|arc| (*arc).clone()),
    );
    BytecodeVerifier::verify(&program)?;
    Ok(Arc::new(program))
}

pub fn compile_file(path: impl AsRef<Path>) -> Result<Arc<Program>> {
    let path = path
        .as_ref()
        .canonicalize()
        .map_err(|error| TinyOneError::compile(format!("File error: {error}")))?;
    let source = fs::read_to_string(&path)
        .map_err(|error| TinyOneError::compile(format!("File error: {error}")))?;
    let shared = Rc::new(RefCell::new(CompilerSharedState::default()));
    let mut compiler = Compiler::new(
        source,
        path.to_string_lossy().to_string(),
        Some(resolve_import),
        false,
        "",
        shared,
    )?;
    let program = compiler.compile()?;
    let program = PeepholeOptimizer::optimize(program);
    BytecodeVerifier::verify(&program)?;
    Ok(Arc::new(program))
}
