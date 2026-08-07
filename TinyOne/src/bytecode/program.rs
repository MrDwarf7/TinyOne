use blake2::{Blake2b512, Digest};

use crate::Instr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub(crate) name: String,
    /// Generic parameters are erased at runtime. They are retained in the
    /// program metadata so v2 tooling and artifact consumers can inspect the
    /// declared parametric API.
    pub(crate) generic_params: Vec<String>,
    pub(crate) param_count: usize,
    pub(crate) code: Vec<Instr>,
    pub(crate) slot_count: usize,
    pub(crate) names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub(crate) name: String,
    pub(crate) fields: Vec<String>,
}

/// One flattened, program-global entry per enum variant. `tag` is the
/// variant's 0-based position within its enum's declaration order; `Op::
/// MakeEnum` indexes this table directly by a flat `variant_id`, so no
/// nested enum-name lookup is needed on the execution hot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantDef {
    pub enum_name: String,
    pub variant_name: String,
    pub tag: u32,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleImportDef {
    pub alias: String,
    pub path: String,
    pub module: String,
    pub resolved: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDef {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) imports: Vec<ModuleImportDef>,
    pub(crate) exported_functions: Vec<String>,
    pub(crate) exported_structs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub(crate) code: Vec<Instr>,
    pub(crate) slot_count: usize,
    pub(crate) names: Vec<String>,
    pub(crate) functions: Vec<Function>,
    pub(crate) strings: Vec<String>,
    pub(crate) structs: Vec<StructDef>,
    pub(crate) fields: Vec<String>,
    pub(crate) modules: Vec<ModuleDef>,
    pub(crate) enum_variants: Vec<EnumVariantDef>,
}

impl Program {
    /// Create a program with empty metadata. The resulting program is still
    /// unverified and must pass through `VerifiedProgram` before execution.
    pub fn new(code: Vec<Instr>, slot_count: usize) -> Self {
        Self {
            code,
            slot_count,
            names: Vec::new(),
            functions: Vec::new(),
            strings: Vec::new(),
            structs: Vec::new(),
            fields: Vec::new(),
            modules: Vec::new(),
            enum_variants: Vec::new(),
        }
    }

    pub fn code(&self) -> &[Instr] {
        &self.code
    }
    pub fn slot_count(&self) -> usize {
        self.slot_count
    }
    pub fn functions(&self) -> &[Function] {
        &self.functions
    }
    pub fn structs(&self) -> &[StructDef] {
        &self.structs
    }
    pub fn modules(&self) -> &[ModuleDef] {
        &self.modules
    }

    pub fn with_functions(mut self, functions: Vec<Function>) -> Self {
        self.functions = functions;
        self
    }

    pub fn with_slot_count(mut self, slot_count: usize) -> Self {
        self.slot_count = slot_count;
        self
    }

    pub fn with_names(mut self, names: Vec<String>) -> Self {
        self.names = names;
        self
    }

    pub fn with_structs(mut self, structs: Vec<StructDef>) -> Self {
        self.structs = structs;
        self
    }

    pub fn fingerprint(&self) -> String {
        let mut hasher = Blake2b512::new();
        self.hash_code(&mut hasher, &self.code);
        hasher.update((self.slot_count as u64).to_le_bytes());
        for name in &self.names {
            hash_string_u32(&mut hasher, name);
        }
        hasher.update((self.functions.len() as u64).to_le_bytes());
        for function in &self.functions {
            hash_string_u32(&mut hasher, &function.name);
            hash_string_list(&mut hasher, function.generic_params.iter());
            hasher.update((function.param_count as u64).to_le_bytes());
            hasher.update((function.slot_count as u64).to_le_bytes());
            self.hash_code(&mut hasher, &function.code);
        }
        for text in &self.strings {
            hash_string_u64(&mut hasher, text);
        }
        for item in &self.structs {
            hash_string_u32(&mut hasher, &item.name);
            hasher.update((item.fields.len() as u32).to_le_bytes());
            for field in &item.fields {
                hash_string_u32(&mut hasher, field);
            }
        }
        for field in &self.fields {
            hash_string_u32(&mut hasher, field);
        }
        for module in &self.modules {
            hash_string_u32(&mut hasher, &module.name);
            hash_string_u32(&mut hasher, &module.path);
            hash_string_list(&mut hasher, module.imports.iter().map(|item| &item.alias));
            hash_string_list(&mut hasher, module.imports.iter().map(|item| &item.path));
            hash_string_list(&mut hasher, module.imports.iter().map(|item| &item.module));
            hash_string_list(
                &mut hasher,
                module.imports.iter().map(|item| &item.resolved),
            );
            hash_string_list(&mut hasher, module.exported_functions.iter());
            hash_string_list(&mut hasher, module.exported_structs.iter());
        }
        let digest = hasher.finalize();
        hex::encode(&digest[..16])
    }

    fn hash_code(&self, hasher: &mut Blake2b512, code: &[Instr]) {
        for instr in code {
            hasher.update(instr.op.ordinal().to_le_bytes());
            hasher.update((instr.arg as i128).to_le_bytes());
            hasher.update((instr.arg2 as i128).to_le_bytes());
        }
    }
}

impl Function {
    pub fn new(
        name: impl Into<String>,
        param_count: usize,
        code: Vec<Instr>,
        slot_count: usize,
    ) -> Self {
        Self {
            name: name.into(),
            generic_params: Vec::new(),
            param_count,
            code,
            slot_count,
            names: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn generic_params(&self) -> &[String] {
        &self.generic_params
    }
    pub fn code(&self) -> &[Instr] {
        &self.code
    }
}

impl StructDef {
    pub fn new(name: impl Into<String>, fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            fields,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn fields(&self) -> &[String] {
        &self.fields
    }
}

impl ModuleDef {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn exported_functions(&self) -> &[String] {
        &self.exported_functions
    }
    pub fn exported_structs(&self) -> &[String] {
        &self.exported_structs
    }
}

fn hash_string_u32(hasher: &mut Blake2b512, value: &str) {
    let bytes = value.as_bytes();
    hasher.update((bytes.len() as u32).to_le_bytes());
    hasher.update(bytes);
}

fn hash_string_u64(hasher: &mut Blake2b512, value: &str) {
    let bytes = value.as_bytes();
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn hash_string_list<'a, I>(hasher: &mut Blake2b512, items: I)
where
    I: ExactSizeIterator<Item = &'a String>,
{
    hasher.update((items.len() as u32).to_le_bytes());
    for item in items {
        hash_string_u32(hasher, item);
    }
}

/// A `Program` that has been validated by `BytecodeVerifier`.
///
/// Construct via `VerifiedProgram::verify(program)` to guarantee the
/// verification ran. Public execution APIs accept `&VerifiedProgram` or
/// `&Program` (with internal re-verification) — this type is provided for
/// callers that want to verify once and reuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedProgram(std::sync::Arc<Program>);

impl VerifiedProgram {
    /// Verify `program` and wrap it. Returns `Err` if verification fails.
    pub fn verify(program: Program) -> crate::Result<Self> {
        crate::BytecodeVerifier::verify(&program)?;
        Ok(Self(std::sync::Arc::new(program)))
    }

    pub(crate) fn verify_arc(program: std::sync::Arc<Program>) -> crate::Result<Self> {
        crate::BytecodeVerifier::verify(&program)?;
        Ok(Self(program))
    }

    /// Borrow the inner program.
    pub fn program(&self) -> &Program {
        &self.0
    }

    pub(crate) fn program_arc(&self) -> std::sync::Arc<Program> {
        std::sync::Arc::clone(&self.0)
    }
}
