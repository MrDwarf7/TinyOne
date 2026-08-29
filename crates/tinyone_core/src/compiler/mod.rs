pub(crate) mod incremental;
pub(crate) mod modules;
pub(crate) mod parser;
pub(crate) mod state;
pub(crate) mod symbols;

pub(crate) use incremental::patch_module;
pub(crate) use modules::{
    ModuleResolver,
    Resolver,
    ResolverInput,
    content_digest,
    default_import_alias,
    module_name_from_import,
    read_source_file,
    unique_module_name,
};
pub(crate) use parser::Compiler;
pub(crate) use state::{CompilerSharedState, ModuleInfo, SharedState};
pub(crate) use symbols::SymbolTable;
