use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::{Function, ModuleDef, ModuleImportDef, StructDef};

#[derive(Debug, Clone)]
pub(crate) struct ModuleInfo {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) function_exports: HashMap<String, usize>,
    pub(crate) struct_exports: HashMap<String, usize>,
    pub(crate) all_functions: HashSet<String>,
    pub(crate) all_structs: HashSet<String>,
    pub(crate) imports: Vec<ModuleImportDef>,
    pub(crate) finalized: bool,
}

#[derive(Debug, Default)]
pub(crate) struct CompilerSharedState {
    pub(crate) function_indexes: HashMap<String, usize>,
    pub(crate) functions: Vec<Function>,
    pub(crate) struct_indexes: HashMap<String, usize>,
    pub(crate) structs: Vec<StructDef>,
    pub(crate) field_indexes: HashMap<String, usize>,
    pub(crate) fields: Vec<String>,
    pub(crate) string_indexes: HashMap<String, usize>,
    pub(crate) strings: Vec<String>,
    pub(crate) modules: HashMap<String, ModuleInfo>,
    pub(crate) loading_modules: HashSet<String>,
    pub(crate) module_defs: Vec<ModuleDef>,
    pub(crate) module_name_owners: HashMap<String, String>,
}

pub(crate) type SharedState = Rc<RefCell<CompilerSharedState>>;
