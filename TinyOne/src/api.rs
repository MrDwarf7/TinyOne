use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    CompileCacheStatus, Compiler, CompilerSharedState, Lexer, ModuleResolver, PeepholeOptimizer,
    Program, Result, TinyOneError, VerifiedProgram, compile_cache, read_source_file,
};

pub fn compile_source(source: &str) -> Result<Arc<Program>> {
    Ok(compile_source_verified(source)?.program_arc())
}

pub fn compile_source_verified(source: &str) -> Result<VerifiedProgram> {
    compile_source_verified_with_filename(source, "<source>")
}

pub fn lex_source(source: &str) -> Result<usize> {
    Ok(Lexer::new(source, "<source>").tokenize()?.len())
}

pub fn compile_source_unoptimized(source: &str) -> Result<Arc<Program>> {
    Ok(compile_source_unoptimized_verified(source)?.program_arc())
}

pub fn compile_source_unoptimized_verified(source: &str) -> Result<VerifiedProgram> {
    compile_source_unoptimized_verified_with_filename(source, "<source>")
}

pub fn compile_source_unoptimized_with_filename(
    source: &str,
    filename: &str,
) -> Result<Arc<Program>> {
    Ok(compile_source_unoptimized_verified_with_filename(source, filename)?.program_arc())
}

pub fn compile_source_unoptimized_verified_with_filename(
    source: &str,
    filename: &str,
) -> Result<VerifiedProgram> {
    VerifiedProgram::verify(compile_source_unoptimized_program(source, filename)?)
}

fn compile_source_unoptimized_program(source: &str, filename: &str) -> Result<Program> {
    let shared = Rc::new(RefCell::new(CompilerSharedState::default()));
    let mut compiler = Compiler::new(source, filename, None, false, "", shared)?;
    compiler.compile()
}

pub fn optimize_program(program: Arc<Program>) -> Arc<Program> {
    Arc::new(PeepholeOptimizer::optimize(
        Arc::try_unwrap(program).unwrap_or_else(|arc| (*arc).clone()),
    ))
}

pub fn compile_source_with_filename(source: &str, filename: &str) -> Result<Arc<Program>> {
    Ok(compile_source_verified_with_filename(source, filename)?.program_arc())
}

pub fn compile_source_verified_with_filename(
    source: &str,
    filename: &str,
) -> Result<VerifiedProgram> {
    let program =
        PeepholeOptimizer::optimize(compile_source_unoptimized_program(source, filename)?);
    VerifiedProgram::verify(program)
}

pub fn compile_file(path: impl AsRef<Path>) -> Result<Arc<Program>> {
    Ok(compile_file_verified(path)?.program_arc())
}

pub fn compile_file_verified(path: impl AsRef<Path>) -> Result<VerifiedProgram> {
    compile_file_verified_with_options(path, true)
}

pub fn compile_file_cached(path: impl AsRef<Path>) -> Result<Arc<Program>> {
    Ok(compile_file_cached_verified(path)?.program_arc())
}

pub fn compile_file_cached_verified(path: impl AsRef<Path>) -> Result<VerifiedProgram> {
    compile_file_cached_verified_with_status(path).map(|(program, _)| program)
}

pub fn compile_file_cached_verified_with_status(
    path: impl AsRef<Path>,
) -> Result<(VerifiedProgram, CompileCacheStatus)> {
    compile_file_cached_verified_with_options(path, true)
}

pub fn compile_file_unoptimized(path: impl AsRef<Path>) -> Result<Arc<Program>> {
    Ok(compile_file_unoptimized_verified(path)?.program_arc())
}

pub fn compile_file_unoptimized_verified(path: impl AsRef<Path>) -> Result<VerifiedProgram> {
    compile_file_verified_with_options(path, false)
}

fn compile_file_verified_with_options(
    path: impl AsRef<Path>,
    optimize: bool,
) -> Result<VerifiedProgram> {
    let path = path
        .as_ref()
        .canonicalize()
        .map_err(|error| TinyOneError::compile(format!("File error: {error}")))?;
    compile_canonical_file(&path, optimize).map(|(program, _, _)| program)
}

pub fn compile_file_cached_verified_with_options(
    path: impl AsRef<Path>,
    optimize: bool,
) -> Result<(VerifiedProgram, CompileCacheStatus)> {
    let path = path
        .as_ref()
        .canonicalize()
        .map_err(|error| TinyOneError::compile(format!("File error: {error}")))?;
    if compile_cache::should_bypass_filesystem(&path) {
        let (program, _, _) = compile_canonical_file(&path, optimize)?;
        return Ok((program, CompileCacheStatus::Bypassed));
    }
    if compile_cache::is_known_bypass(&path, optimize) {
        let (program, _, _) = compile_canonical_file(&path, optimize)?;
        return Ok((program, CompileCacheStatus::Bypassed));
    }
    match compile_cache::lookup(&path, optimize)? {
        compile_cache::CacheLookup::Hit(program) => {
            return Ok((program, CompileCacheStatus::Hit));
        }
        compile_cache::CacheLookup::Incremental(mut candidate) => {
            let module_source = candidate
                .module_source
                .take()
                .ok_or_else(|| TinyOneError::compile("Incremental cache source missing"))?;
            if let Ok((replacement, resolver)) = compile_module_fragment(
                &candidate.module_path,
                &candidate.module_name,
                module_source,
                optimize,
            ) {
                let cached = candidate
                    .program
                    .take()
                    .ok_or_else(|| TinyOneError::compile("Incremental cache program missing"))?;
                if let Ok(program) =
                    crate::patch_module(cached, &replacement, &candidate.module_name)
                {
                    let _ = compile_cache::store_incremental(
                        &path,
                        optimize,
                        &program,
                        *candidate,
                        &resolver.borrow(),
                    );
                    return Ok((program, CompileCacheStatus::Incremental));
                }
            }
        }
        compile_cache::CacheLookup::Bypass => {
            let (program, _, _) = compile_canonical_file(&path, optimize)?;
            compile_cache::remember_bypass(&path, optimize);
            return Ok((program, CompileCacheStatus::Bypassed));
        }
        compile_cache::CacheLookup::Miss => {}
    }
    let (program, source, resolver) = compile_canonical_file(&path, optimize)?;
    if compile_cache::should_bypass(&source, &resolver.borrow()) {
        compile_cache::remember_bypass(&path, optimize);
        return Ok((program, CompileCacheStatus::Bypassed));
    }
    // A cache write is an optimization. Read-only source trees and transient
    // cache failures must not prevent a correctly compiled program from
    // running.
    let _ = compile_cache::store(&path, &source, optimize, &program, &resolver.borrow());
    Ok((program, CompileCacheStatus::Miss))
}

fn compile_module_fragment(
    path: &Path,
    module_name: &str,
    source: String,
    optimize: bool,
) -> Result<(Program, Rc<RefCell<ModuleResolver>>)> {
    let shared = Rc::new(RefCell::new(CompilerSharedState::default()));
    let resolver = Rc::new(RefCell::new(ModuleResolver::default()));
    let mut compiler = Compiler::new(
        source,
        path.to_string_lossy().to_string(),
        Some(Rc::clone(&resolver)),
        true,
        module_name,
        shared,
    )?;
    let program = compiler.compile()?;
    let program = if optimize {
        PeepholeOptimizer::optimize(program)
    } else {
        program
    };
    Ok((program, resolver))
}

fn compile_canonical_file(
    path: &Path,
    optimize: bool,
) -> Result<(VerifiedProgram, String, Rc<RefCell<ModuleResolver>>)> {
    let source = read_source_file(path)?;
    let shared = Rc::new(RefCell::new(CompilerSharedState::default()));
    let resolver = Rc::new(RefCell::new(ModuleResolver::default()));
    let mut compiler = Compiler::new(
        source.clone(),
        path.to_string_lossy().to_string(),
        Some(Rc::clone(&resolver)),
        false,
        "",
        shared,
    )?;
    let program = compiler.compile()?;
    let program = if optimize {
        PeepholeOptimizer::optimize(program)
    } else {
        program
    };
    Ok((VerifiedProgram::verify(program)?, source, resolver))
}
