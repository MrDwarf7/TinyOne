mod api;
mod artifact_io;
mod builtins;
mod bytecode;
mod compiler;
mod error;
mod ffi;
#[cfg(any(test, feature = "testing-hooks"))]
mod internal_testing;
mod jit;
mod runner;
mod runtime;
mod source;
mod syntax;
#[cfg(feature = "testing-hooks")]
pub mod testing;

pub use api::{
    compile_file, compile_source, compile_source_unoptimized,
    compile_source_unoptimized_with_filename, compile_source_with_filename, lex_source,
    optimize_program,
};
pub use artifact_io::{load_artifact, write_artifact};
pub(crate) use builtins::{BUILTINS, builtin_index};
pub use bytecode::{
    BytecodeVerifier, Function, Instr, ModuleDef, ModuleImportDef, Op, PeepholeOptimizer, Program,
    StructDef,
};
pub(crate) use compiler::{
    Compiler, CompilerSharedState, ModuleInfo, Resolver, SharedState, SymbolTable,
    default_import_alias, module_name_from_import, resolve_import, unique_module_name,
};
pub use error::{Result, TinyOneError};
pub(crate) use jit::{HOT_BACK_EDGE_THRESHOLD, JitChunk, JitFunction, JitOp, JitVm};
pub use jit::{JitCache, JitCacheStats, JitProgram, JitStats, write_jit_listing};
pub use runner::{
    run_program, run_program_report, run_program_with_env, run_source, run_source_report,
};
pub(crate) use runtime::{
    HeapData, MAX_ARRAY_LENGTH, MAX_BUFFER_BYTES, MAX_CALL_DEPTH, MAX_HEAP_BYTES, MAX_HEAP_OBJECTS,
    TinyHeap, TinyRuntimeContext, VALUE_BYTES, Value, checked_bounded_len, checked_byte_range,
    checked_collection_index, checked_div, checked_div_int, checked_non_negative_usize,
    checked_payload_bytes, expect_int, expect_pointer, expect_string, floor_div, pop_args,
    runtime_add, runtime_add_int, runtime_array_pop, runtime_array_push, runtime_call_builtin,
    runtime_cast_pointer, runtime_compare, runtime_compare_int, runtime_get_field, runtime_index,
    runtime_is_false, runtime_make_array, runtime_make_buffer, runtime_make_field_pointer,
    runtime_make_pointer, runtime_make_struct, runtime_mul, runtime_mul_int, runtime_neg,
    runtime_null, runtime_pointer_add, runtime_pointer_address, runtime_pointer_at,
    runtime_pointer_base, runtime_pointer_eq, runtime_pointer_field, runtime_pointer_kind,
    runtime_pointer_load, runtime_pointer_offset, runtime_pointer_store, runtime_pointer_type,
    runtime_print, runtime_read_uint, runtime_set_field, runtime_set_index, runtime_sub,
    runtime_sub_int, runtime_write_uint, validate_pointer_base,
};
pub use runtime::{
    HeapRef, RawPointer, RuntimeValue, TinyHeapStats, TinyMemory, TinyRunReport, TypeKind, VM,
};
pub(crate) use source::SourceMap;
pub(crate) use syntax::{Lexer, Token, TokenKind};
