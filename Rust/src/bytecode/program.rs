use blake2::{Blake2b512, Digest};

use crate::Instr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub param_count: usize,
    pub code: Vec<Instr>,
    pub slot_count: usize,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub name: String,
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
    pub name: String,
    pub path: String,
    pub imports: Vec<ModuleImportDef>,
    pub exported_functions: Vec<String>,
    pub exported_structs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub code: Vec<Instr>,
    pub slot_count: usize,
    pub names: Vec<String>,
    pub functions: Vec<Function>,
    pub strings: Vec<String>,
    pub structs: Vec<StructDef>,
    pub fields: Vec<String>,
    pub modules: Vec<ModuleDef>,
}

impl Program {
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
            let lists: [&[String]; 6] = [
                &module
                    .imports
                    .iter()
                    .map(|item| item.alias.clone())
                    .collect::<Vec<_>>(),
                &module
                    .imports
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<Vec<_>>(),
                &module
                    .imports
                    .iter()
                    .map(|item| item.module.clone())
                    .collect::<Vec<_>>(),
                &module
                    .imports
                    .iter()
                    .map(|item| item.resolved.clone())
                    .collect::<Vec<_>>(),
                &module.exported_functions,
                &module.exported_structs,
            ];
            for list in lists {
                hasher.update((list.len() as u32).to_le_bytes());
                for item in list {
                    hash_string_u32(&mut hasher, item);
                }
            }
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
