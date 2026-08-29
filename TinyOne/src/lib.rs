pub mod alloc_table;
mod api;
mod artifact_io;
mod builtins;
mod bytecode;
mod compile_cache;
mod compiler;
mod config;
mod error;
mod ffi;
#[cfg(any(test, feature = "testing-hooks"))]
mod internal_testing;
mod jit;
pub mod memory_log;
mod runner;
mod runtime;
mod source;
mod syntax;
#[cfg(feature = "testing-hooks")]
pub mod testing;
pub mod tiny_allocator;
pub mod vm_hooks;

pub use api::{
    compile_file, compile_file_cached, compile_file_cached_verified,
    compile_file_cached_verified_with_options, compile_file_cached_verified_with_status,
    compile_file_unoptimized, compile_file_unoptimized_verified, compile_file_verified,
    compile_source, compile_source_unoptimized, compile_source_unoptimized_verified,
    compile_source_unoptimized_verified_with_filename, compile_source_unoptimized_with_filename,
    compile_source_verified, compile_source_verified_with_filename, compile_source_with_filename,
    lex_source, optimize_program,
};
pub use artifact_io::{
    load_artifact, load_verified_artifact, write_artifact, write_binary_artifact,
};
pub(crate) use builtins::{BUILTINS, builtin_index};
pub use bytecode::{
    BytecodeVerifier, EnumVariantDef, Function, Instr, ModuleDef, ModuleImportDef, Op,
    PeepholeOptimizer, Program, StructDef, VerifiedProgram,
};
pub(crate) use bytecode::{ModuleCapabilities, ModuleCapability, ModulePermissions};
pub use compile_cache::CompileCacheStatus;
pub(crate) use compiler::{
    Compiler, CompilerSharedState, ModuleInfo, ModuleResolver, Resolver, ResolverInput,
    SharedState, SymbolTable, content_digest, default_import_alias, module_name_from_import,
    patch_module, read_source_file, unique_module_name,
};
pub(crate) use config::ProjectConfig;
pub use config::{
    authority_certificate_digest, authority_certificate_payload,
    canonical_module_signature_payload, module_dependency_lock_hash, module_signature_digest,
    module_source_hash,
};
pub use error::{Result, TinyOneError};
#[doc(hidden)]
pub use ffi::sandbox_worker_main;
pub use jit::{
    DEFAULT_HOT_BACK_EDGE_THRESHOLD, DEFAULT_SOURCE_CACHE_MAX_BYTES,
    DEFAULT_SOURCE_CACHE_MAX_ENTRIES, JitCache, JitCacheStats, JitExecutionProfile,
    JitOpcodeProfile, JitOptions, JitProgram, JitStats, write_jit_listing,
    write_verified_jit_listing,
};
pub(crate) use jit::{JitBuiltin, JitChunk, JitOp, JitVm};
pub use runner::{
    run_program, run_program_report, run_program_with_env, run_program_with_env_and_jit_options,
    run_program_with_jit_options, run_source, run_source_report, run_verified_program,
    run_verified_program_report, run_verified_program_with_env,
    run_verified_program_with_env_and_jit_options, run_verified_program_with_jit_options,
};
pub(crate) use runtime::{
    HeapData, MAX_ARRAY_LENGTH, MAX_BUFFER_BYTES, MAX_CALL_DEPTH, MAX_HEAP_BYTES, MAX_HEAP_OBJECTS,
    TinyHeap, TinyRuntimeContext, VALUE_BYTES, Value, VmSettings, checked_bounded_len,
    checked_byte_range, checked_collection_index, checked_div, checked_div_int,
    checked_non_negative_usize, checked_payload_bytes, expect_int, expect_pointer, expect_string,
    floor_div, integer_value_from_kind, pop_args, require_builtin_capability, round_to_kind,
    runtime_add, runtime_add_int, runtime_array_pop, runtime_array_push, runtime_call_builtin,
    runtime_cast_int, runtime_cast_pointer, runtime_compare, runtime_compare_int,
    runtime_get_field, runtime_index, runtime_integer_kind, runtime_integer_value,
    runtime_is_false, runtime_make_array, runtime_make_buffer, runtime_make_enum,
    runtime_make_field_pointer, runtime_make_pointer, runtime_make_struct, runtime_mul,
    runtime_mul_int, runtime_neg, runtime_null, runtime_pointer_add, runtime_pointer_address,
    runtime_pointer_at, runtime_pointer_base, runtime_pointer_eq, runtime_pointer_field,
    runtime_pointer_kind, runtime_pointer_load, runtime_pointer_offset, runtime_pointer_store,
    runtime_pointer_type, runtime_print, runtime_read_uint, runtime_set_field, runtime_set_index,
    runtime_sub, runtime_sub_int, runtime_write_uint, validate_pointer_base,
};
pub use runtime::{
    HeapRef, RawPointer, RuntimeValue, TinyHeapStats, TinyMemory, TinyRunReport, TypeKind, VM,
};
pub(crate) use source::SourceMap;
pub(crate) use syntax::{Lexer, Token, TokenKind};
