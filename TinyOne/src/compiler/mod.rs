pub(crate) mod modules;
pub(crate) mod parser;
pub(crate) mod state;
pub(crate) mod symbols;

pub(crate) use modules::{
    Resolver, default_import_alias, module_name_from_import, resolve_import, unique_module_name,
};
pub(crate) use parser::Compiler;
pub(crate) use state::{CompilerSharedState, ModuleInfo, SharedState};
pub(crate) use symbols::SymbolTable;
