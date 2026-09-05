use crate::{
    EnumVariantDef,
    Function,
    Instr,
    ModuleCapabilities,
    ModuleDef,
    ModuleImportDef,
    ModulePermissions,
    Op,
    Program,
    Result,
    StructDef,
    TinyOneError,
    VerifiedProgram,
    VmSettings,
};

pub(crate) const BINARY_ARTIFACT_MAGIC: &[u8; 8] = b"TINYONEB";
pub(crate) const MAX_BINARY_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
// Version 4 adds the fine-grained policy which backs signed module grants.
const BINARY_ARTIFACT_VERSION: u16 = 4;
const MAX_FUNCTIONS: usize = 4_096;
const MAX_STRUCTS: usize = 4_096;
const MAX_CODE_OPS: usize = 65_536;
const MAX_TOTAL_CODE_OPS: usize = 262_144;
const MAX_STRINGS: usize = 65_536;
const MAX_FIELDS: usize = 65_536;
const MAX_SLOT_COUNT: usize = 65_536;
const MAX_MODULES: usize = 256;
const MAX_MODULE_IMPORTS: usize = 4_096;
const MAX_MODULE_EXPORTS: usize = 4_096;
const MAX_STRUCT_FIELDS: usize = 256;
const MAX_ENUM_VARIANTS: usize = 4_096;
const MAX_ENUM_FIELDS: usize = 256;
const MAX_NAMES: usize = 65_536;
const MAX_PERMISSION_ENVIRONMENT_VARIABLES: usize = 4_096;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

impl Program {
    /// Encodes this program into its binary artifact representation.
    ///
    /// # Errors
    ///
    /// Returns an error if any field cannot be encoded within the binary
    /// format's size limits (for example if a string, string list, or
    /// instruction count exceeds `u32::MAX`), or if the encoded artifact
    /// exceeds [`MAX_BINARY_ARTIFACT_BYTES`].
    pub fn to_binary_artifact(&self) -> Result<Vec<u8>> {
        let mut writer = BinaryWriter::default();
        writer.bytes.extend_from_slice(BINARY_ARTIFACT_MAGIC);
        writer.u16(BINARY_ARTIFACT_VERSION);
        writer.u8(self.root_capabilities.bits());
        writer.permissions(&self.root_permissions)?;
        writer.usize(self.vm_settings.max_call_depth, "VM max call depth")?;
        writer.code(&self.code)?;
        writer.usize(self.slot_count, "main slot count")?;
        writer.string_list(&self.names)?;
        writer.usize(self.functions.len(), "function count")?;
        for function in &self.functions {
            writer.string(&function.name)?;
            writer.string_list(&function.generic_params)?;
            writer.usize(function.param_count, "function parameter count")?;
            writer.code(&function.code)?;
            writer.usize(function.slot_count, "function slot count")?;
            writer.string_list(&function.names)?;
        }
        writer.string_list(&self.strings)?;
        writer.usize(self.structs.len(), "struct count")?;
        for item in &self.structs {
            writer.string(&item.name)?;
            writer.string_list(&item.fields)?;
        }
        writer.string_list(&self.fields)?;
        writer.usize(self.modules.len(), "module count")?;
        for module in &self.modules {
            writer.string(&module.name)?;
            writer.string(&module.path)?;
            writer.usize(module.imports.len(), "module import count")?;
            for import in &module.imports {
                writer.string(&import.alias)?;
                writer.string(&import.path)?;
                writer.string(&import.module)?;
                writer.string(&import.resolved)?;
            }
            writer.string_list(&module.exported_functions)?;
            writer.string_list(&module.exported_structs)?;
            writer.u8(module.capabilities.bits());
            writer.permissions(&module.permissions)?;
        }
        writer.usize(self.enum_variants.len(), "enum variant count")?;
        for item in &self.enum_variants {
            writer.string(&item.enum_name)?;
            writer.string(&item.variant_name)?;
            writer.u32(item.tag);
            writer.string_list(&item.fields)?;
        }
        if writer.bytes.len() > MAX_BINARY_ARTIFACT_BYTES {
            return Err(TinyOneError::compile(format!(
                "Binary artifact exceeds byte size limit {MAX_BINARY_ARTIFACT_BYTES}"
            )));
        }
        Ok(writer.bytes)
    }

    /// Decodes an untrusted binary artifact and verifies it, returning a
    /// [`Program`].
    ///
    /// # Errors
    ///
    /// Returns an error if `bytes` is not a valid binary artifact (exceeds the
    /// byte size limit, has an invalid magic header or version, or is truncated
    /// or malformed), or if bytecode verification fails.
    pub fn from_binary_artifact(bytes: &[u8]) -> Result<Self> {
        VerifiedProgram::from_binary_artifact(bytes).map(VerifiedProgram::into_program)
    }

    /// Decodes policy-bearing binary data after the caller has authenticated
    /// the artifact bytes and accepted its authority as an embedding decision.
    ///
    /// # Errors
    ///
    /// Returns an error if `bytes` is not a valid binary artifact (exceeds the
    /// byte size limit, has an invalid magic header or version, or is truncated
    /// or malformed), or if bytecode verification fails.
    pub fn from_trusted_binary_artifact(bytes: &[u8]) -> Result<Self> {
        VerifiedProgram::from_trusted_binary_artifact(bytes).map(VerifiedProgram::into_program)
    }

    pub(crate) fn decode_binary_artifact(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BINARY_ARTIFACT_BYTES {
            return Err(TinyOneError::compile(format!(
                "Binary artifact rejected: byte size limit {MAX_BINARY_ARTIFACT_BYTES} exceeded"
            )));
        }
        let mut reader = BinaryReader::new(bytes);
        if reader.take(BINARY_ARTIFACT_MAGIC.len())? != BINARY_ARTIFACT_MAGIC {
            return Err(TinyOneError::compile("Unsupported TinyOne binary artifact magic"));
        }
        let version = reader.u16()?;
        if version != BINARY_ARTIFACT_VERSION {
            return Err(TinyOneError::compile(format!("Unsupported TinyOne binary artifact version {version}")));
        }
        let root_capabilities = ModuleCapabilities::from_bits(reader.u8()?)?;
        let root_permissions = reader.permissions(root_capabilities, "root permissions")?;
        let vm_settings =
            VmSettings::with_max_call_depth(reader.bounded_usize("VM max call depth", crate::MAX_CALL_DEPTH)?)?;
        let code = reader.code("main code")?;
        let slot_count = reader.bounded_usize("main slot count", MAX_SLOT_COUNT)?;
        let names = reader.string_list("main names", MAX_NAMES)?;
        let function_count = reader.bounded_usize("function count", MAX_FUNCTIONS)?;
        let mut functions = Vec::with_capacity(function_count);
        for _ in 0..function_count {
            let name = reader.string("function name")?;
            let generic_params = reader.string_list("generic parameters", MAX_NAMES)?;
            let param_count = reader.bounded_usize("parameter count", MAX_SLOT_COUNT)?;
            let function_code = reader.code("function code")?;
            let function_slot_count = reader.bounded_usize("function slot count", MAX_SLOT_COUNT)?;
            let function_names = reader.string_list("function names", MAX_NAMES)?;
            functions.push(Function {
                name,
                generic_params,
                param_count,
                code: function_code,
                slot_count: function_slot_count,
                names: function_names,
            });
        }
        let strings = reader.string_list("strings", MAX_STRINGS)?;
        let struct_count = reader.bounded_usize("struct count", MAX_STRUCTS)?;
        let mut structs = Vec::with_capacity(struct_count);
        for _ in 0..struct_count {
            structs.push(StructDef {
                name:   reader.string("struct name")?,
                fields: reader.string_list("struct fields", MAX_STRUCT_FIELDS)?,
            });
        }
        let fields = reader.string_list("fields", MAX_FIELDS)?;
        let module_count = reader.bounded_usize("module count", MAX_MODULES)?;
        let mut modules = Vec::with_capacity(module_count);
        for _ in 0..module_count {
            let name = reader.string("module name")?;
            let path = reader.string("module path")?;
            let import_count = reader.bounded_usize("module import count", MAX_MODULE_IMPORTS)?;
            let mut imports = Vec::with_capacity(import_count);
            for _ in 0..import_count {
                imports.push(ModuleImportDef {
                    alias:    reader.string("module import alias")?,
                    path:     reader.string("module import path")?,
                    module:   reader.string("module import target")?,
                    resolved: reader.string("resolved module import target")?,
                });
            }
            let exported_functions = reader.string_list("module function exports", MAX_MODULE_EXPORTS)?;
            let exported_structs = reader.string_list("module struct exports", MAX_MODULE_EXPORTS)?;
            let capabilities = ModuleCapabilities::from_bits(reader.u8()?)?;
            let permissions = reader.permissions(capabilities, "module permissions")?;
            modules.push(ModuleDef {
                name,
                path,
                imports,
                exported_functions,
                exported_structs,
                capabilities,
                permissions,
            });
        }
        let enum_count = reader.bounded_usize("enum variant count", MAX_ENUM_VARIANTS)?;
        let mut enum_variants = Vec::with_capacity(enum_count);
        for _ in 0..enum_count {
            enum_variants.push(EnumVariantDef {
                enum_name:    reader.string("enum name")?,
                variant_name: reader.string("enum variant name")?,
                tag:          reader.u32()?,
                fields:       reader.string_list("enum variant fields", MAX_ENUM_FIELDS)?,
            });
        }
        if !reader.is_empty() {
            return Err(TinyOneError::compile("Binary artifact contains trailing data"));
        }
        Ok(Self {
            code,
            slot_count,
            names,
            functions,
            strings,
            structs,
            fields,
            modules,
            enum_variants,
            root_capabilities,
            root_permissions,
            policy_trusted: false,
            vm_settings,
        })
    }
}

impl VerifiedProgram {
    /// Decodes an untrusted binary artifact without granting its serialized
    /// host authority. Use [`Self::from_trusted_binary_artifact`] only after
    /// the embedding application authenticates the bytes and policy.
    ///
    /// # Errors
    ///
    /// Returns an error if `bytes` is not a valid binary artifact (exceeds the
    /// byte size limit, has an invalid magic header or version, or is truncated
    /// or malformed), or if bytecode verification fails.
    pub fn from_binary_artifact(bytes: &[u8]) -> Result<Self> {
        Self::verify(Program::decode_binary_artifact(bytes)?)
    }

    /// Decodes a binary artifact whose bytes and policy were authenticated by
    /// the embedding application.
    ///
    /// # Errors
    ///
    /// Returns an error if `bytes` is not a valid binary artifact (exceeds the
    /// byte size limit, has an invalid magic header or version, or is truncated
    /// or malformed), or if bytecode verification fails.
    pub fn from_trusted_binary_artifact(bytes: &[u8]) -> Result<Self> {
        Self::verify(Program::decode_binary_artifact(bytes)?.trust_artifact_policy())
    }
}

#[derive(Default)]
struct BinaryWriter {
    bytes: Vec<u8>,
}

impl BinaryWriter {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn usize(&mut self, value: usize, name: &str) -> Result<()> {
        let value =
            u32::try_from(value).map_err(|_| TinyOneError::compile(format!("Binary artifact {name} is too large")))?;
        self.u32(value);
        Ok(())
    }

    fn string(&mut self, value: &str) -> Result<()> {
        self.usize(value.len(), "string")?;
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    fn string_list(&mut self, values: &[String]) -> Result<()> {
        self.usize(values.len(), "string list")?;
        for value in values {
            self.string(value)?;
        }
        Ok(())
    }

    fn code(&mut self, code: &[Instr]) -> Result<()> {
        self.usize(code.len(), "instruction count")?;
        for instruction in code {
            self.u16(instruction.op.ordinal());
            self.i64(instruction.arg);
            self.i64(instruction.arg2);
        }
        Ok(())
    }

    fn permissions(&mut self, permissions: &ModulePermissions) -> Result<()> {
        self.u8(u8::from(permissions.allows_filesystem_read()));
        self.u8(u8::from(permissions.allows_filesystem_write()));
        match permissions.environment_read_allowlist() {
            None => self.u8(0),
            Some(values) => {
                self.u8(1);
                self.string_list(values)?;
            }
        }
        self.u8(u8::from(permissions.network_outbound()));
        self.u8(u8::from(permissions.network_listen()));
        self.u8(u8::from(permissions.process_spawn()));
        self.u8(u8::from(permissions.ffi_allowed()));
        self.u8(u8::from(permissions.graphics_gpu()));
        self.u8(u8::from(permissions.hardware_access()));
        self.u8(u8::from(permissions.threads_allowed()));
        self.u8(u8::from(permissions.unsafe_memory_allowed()));
        self.u8(u8::from(permissions.linux_pipelines_allowed()));
        Ok(())
    }
}

struct BinaryReader<'a> {
    bytes:          &'a [u8],
    offset:         usize,
    total_code_ops: usize,
}

impl<'a> BinaryReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            total_code_ops: 0,
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| TinyOneError::compile("Binary artifact offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| TinyOneError::compile("Binary artifact is truncated"))?;
        self.offset = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| TinyOneError::compile("Binary artifact is truncated"))?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u8(&mut self) -> Result<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| TinyOneError::compile("Binary artifact is truncated"))?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn i64(&mut self) -> Result<i64> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| TinyOneError::compile("Binary artifact is truncated"))?;
        Ok(i64::from_le_bytes(bytes))
    }

    fn bounded_usize(&mut self, name: &str, max: usize) -> Result<usize> {
        let value = self.u32()? as usize;
        if value > max {
            return Err(TinyOneError::compile(format!(
                "Binary artifact rejected: {name} limit {max} exceeded (got {value})"
            )));
        }
        Ok(value)
    }

    fn string(&mut self, name: &str) -> Result<String> {
        let length = self.bounded_usize(&format!("{name} bytes"), MAX_TEXT_BYTES)?;
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|error| TinyOneError::compile(format!("Binary artifact {name} must be UTF-8: {error}")))
    }

    fn string_list(&mut self, name: &str, max: usize) -> Result<Vec<String>> {
        let count = self.bounded_usize(name, max)?;
        let mut values = Vec::with_capacity(count);
        let mut total_bytes = 0usize;
        for _ in 0..count {
            let value = self.string(name)?;
            total_bytes = total_bytes
                .checked_add(value.len())
                .ok_or_else(|| TinyOneError::compile(format!("Binary artifact {name} text overflow")))?;
            if total_bytes > MAX_TEXT_BYTES {
                return Err(TinyOneError::compile(format!(
                    "Binary artifact rejected: {name} text limit {MAX_TEXT_BYTES} exceeded"
                )));
            }
            values.push(value);
        }
        Ok(values)
    }

    fn code(&mut self, name: &str) -> Result<Vec<Instr>> {
        let count = self.bounded_usize(name, MAX_CODE_OPS)?;
        self.total_code_ops = self
            .total_code_ops
            .checked_add(count)
            .ok_or_else(|| TinyOneError::compile("Binary artifact total instruction count overflow"))?;
        if self.total_code_ops > MAX_TOTAL_CODE_OPS {
            return Err(TinyOneError::compile(format!(
                "Binary artifact rejected: total instruction limit {MAX_TOTAL_CODE_OPS} exceeded"
            )));
        }
        let mut code = Vec::with_capacity(count);
        for _ in 0..count {
            code.push(Instr::new(Op::from_ordinal(self.u16()?)?, self.i64()?, self.i64()?));
        }
        Ok(code)
    }

    fn boolean(&mut self, name: &str) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(TinyOneError::compile(format!("Binary artifact {name} must be encoded as 0 or 1"))),
        }
    }

    fn permissions(&mut self, capabilities: ModuleCapabilities, name: &str) -> Result<ModulePermissions> {
        let filesystem_read = self.boolean(&format!("{name} filesystem read"))?;
        let filesystem_write = self.boolean(&format!("{name} filesystem write"))?;
        let environment_read = match self.u8()? {
            0 => None,
            1 => {
                Some(self.string_list(&format!("{name} environment allowlist"), MAX_PERMISSION_ENVIRONMENT_VARIABLES)?)
            }
            _ => {
                return Err(TinyOneError::compile(format!(
                    "Binary artifact {name} environment allowlist tag must be 0 or 1"
                )));
            }
        };
        ModulePermissions::from_artifact(
            capabilities,
            filesystem_read,
            filesystem_write,
            environment_read,
            self.boolean(&format!("{name} network outbound"))?,
            self.boolean(&format!("{name} network listen"))?,
            self.boolean(&format!("{name} process spawn"))?,
            self.boolean(&format!("{name} ffi"))?,
            self.boolean(&format!("{name} graphics"))?,
            self.boolean(&format!("{name} hardware"))?,
            self.boolean(&format!("{name} threads"))?,
            self.boolean(&format!("{name} unsafe memory"))?,
            self.boolean(&format!("{name} linux pipelines"))?,
        )
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_binary_round_trip_verifies() {
        let program = Program::new(vec![Instr::new(Op::Halt, 0, 0)], 0);
        let bytes = program.to_binary_artifact().expect("encode");
        let decoded = Program::from_trusted_binary_artifact(&bytes).expect("trusted decode");
        assert_eq!(program, decoded);
    }

    #[test]
    fn binary_decoder_rejects_truncation_and_trailing_data() {
        let program = Program::new(vec![Instr::new(Op::Halt, 0, 0)], 0);
        let bytes = program.to_binary_artifact().expect("encode");
        assert!(Program::from_binary_artifact(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(Program::from_binary_artifact(&trailing).is_err());
    }
}
