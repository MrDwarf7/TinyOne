use serde_json::{Value as JsonValue, json};

use crate::{
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

pub(crate) const MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

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
const MAX_MODULE_CAPABILITIES: usize = 8;
const MAX_PERMISSION_ENVIRONMENT_VARIABLES: usize = 4_096;
const MAX_STRUCT_FIELDS: usize = 256;
#[allow(dead_code)]
const MAX_ENUM_VARIANTS: usize = 65_536;
const MAX_NAMES: usize = 65_536;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

// Version 1 did not define persisted execution policy. Keep accepting it as
// a legacy input, but never give newly emitted policy-bearing artifacts that
// version: a v1 reader would silently ignore the new fields.
const LEGACY_JSON_ARTIFACT_VERSION: i64 = 1;
const JSON_ARTIFACT_VERSION: i64 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonArtifactVersion {
    LegacyV1,
    V2,
}

impl Program {
    #[must_use]
    pub fn to_artifact(&self) -> JsonValue {
        json!({
            "format": "tinyone-bytecode",
            "version": JSON_ARTIFACT_VERSION,
            "root_capabilities": self.root_capabilities.names(),
            "root_permissions": encode_permissions(&self.root_permissions),
            "vm": {
                "max_call_depth": self.vm_settings.max_call_depth,
            },
            "code": encode_code(&self.code),
            "slot_count": self.slot_count,
            "names": self.names,
            "functions": self.functions.iter().map(|function| json!({
                "name": function.name,
                "generic_params": function.generic_params,
                "param_count": function.param_count,
                "code": encode_code(&function.code),
                "slot_count": function.slot_count,
                "names": function.names,
            })).collect::<Vec<_>>(),
            "strings": self.strings,
            "structs": self.structs.iter().map(|item| json!({
                "name": item.name,
                "fields": item.fields,
            })).collect::<Vec<_>>(),
            "fields": self.fields,
            "modules": self.modules.iter().map(|module| json!({
                "name": module.name,
                "path": module.path,
                "imports": module.imports.iter().map(|item| json!({
                    "alias": item.alias,
                    "path": item.path,
                    "module": item.module,
                    "resolved": item.resolved,
                })).collect::<Vec<_>>(),
                "exported_functions": module.exported_functions,
                "exported_structs": module.exported_structs,
                "capabilities": module.capabilities.names(),
                "permissions": encode_permissions(&module.permissions),
            })).collect::<Vec<_>>(),
        })
    }

    /// Decodes an untrusted JSON artifact and verifies it, returning a
    /// [`Program`].
    ///
    /// # Errors
    ///
    /// Returns an error if `data` is not a valid `TinyOne` artifact (wrong format
    /// string, unsupported version, or malformed or oversized fields), or if
    /// bytecode verification fails.
    pub fn from_artifact(data: JsonValue) -> Result<Self> {
        VerifiedProgram::from_artifact(data).map(VerifiedProgram::into_program)
    }

    /// Decodes policy-bearing JSON after the caller has authenticated the
    /// artifact bytes and accepted its authority as an embedding decision.
    ///
    /// # Errors
    ///
    /// Returns an error if `data` is not a valid `TinyOne` artifact (wrong format
    /// string, unsupported version, or malformed or oversized fields), or if
    /// bytecode verification fails.
    pub fn from_trusted_artifact(data: JsonValue) -> Result<Self> {
        VerifiedProgram::from_trusted_artifact(data).map(VerifiedProgram::into_program)
    }

    pub(crate) fn decode_artifact(data: JsonValue) -> Result<Self> {
        let object = data
            .as_object()
            .ok_or_else(|| TinyOneError::compile("Artifact must be a JSON object"))?;
        if object.get("format").and_then(JsonValue::as_str) != Some("tinyone-bytecode") {
            return Err(TinyOneError::compile("Unsupported TinyOne artifact format"));
        }
        let version = match object.get("version").and_then(JsonValue::as_i64) {
            Some(LEGACY_JSON_ARTIFACT_VERSION) => JsonArtifactVersion::LegacyV1,
            Some(JSON_ARTIFACT_VERSION) => JsonArtifactVersion::V2,
            _ => return Err(TinyOneError::compile("Unsupported TinyOne artifact format")),
        };
        let raw_functions = expect_array_limited(object.get("functions"), "functions", MAX_FUNCTIONS)?;
        let main_slot_count = expect_usize(object.get("slot_count"), "slot_count")?;
        reject_over_limit("slot_count", main_slot_count, MAX_SLOT_COUNT)?;
        let main_names = expect_string_list_limited(object.get("names"), "names", MAX_NAMES)?;
        let strings = expect_string_list_limited(object.get("strings"), "strings", MAX_STRINGS)?;
        let fields = expect_string_list_limited(object.get("fields"), "fields", MAX_FIELDS)?;
        let raw_structs = expect_array_limited(object.get("structs"), "structs", MAX_STRUCTS)?;
        let raw_modules = optional_array_limited(object.get("modules"), "modules", MAX_MODULES)?;
        let (root_capabilities, root_permissions, vm_settings) = match version {
            // v1 artifacts predate policy serialization. Interpret every v1
            // input using the historical defaults, including malformed or
            // restrictive policy-looking fields that a v1 reader would have
            // ignored. This prevents a v1 payload from claiming a policy that
            // older readers would silently drop.
            JsonArtifactVersion::LegacyV1 => {
                let root_capabilities = ModuleCapabilities::all();
                (root_capabilities, ModulePermissions::from_capabilities(root_capabilities), VmSettings::default())
            }
            JsonArtifactVersion::V2 => {
                let root_capability_names = expect_string_list_limited(
                    object.get("root_capabilities"),
                    "root_capabilities",
                    MAX_MODULE_CAPABILITIES,
                )?;
                let root_capabilities = ModuleCapabilities::from_names(&root_capability_names)?;
                let root_permissions =
                    decode_permissions(object.get("root_permissions"), "root_permissions", root_capabilities)?;
                let vm = object
                    .get("vm")
                    .and_then(JsonValue::as_object)
                    .ok_or_else(|| TinyOneError::compile("Artifact field \"vm\" must be an object"))?;
                let vm_settings =
                    VmSettings::with_max_call_depth(expect_usize(vm.get("max_call_depth"), "vm.max_call_depth")?)?;
                (root_capabilities, root_permissions, vm_settings)
            }
        };
        let mut total_code_ops = 0usize;
        let functions = raw_functions
            .iter()
            .map(|item| {
                let obj = item
                    .as_object()
                    .ok_or_else(|| TinyOneError::compile("Function artifact must be an object"))?;
                let func_code = decode_code_limited(obj.get("code"), "function code")?;
                total_code_ops = total_code_ops
                    .checked_add(func_code.len())
                    .ok_or_else(|| TinyOneError::compile("Artifact rejected: code size overflow"))?;
                reject_over_limit("total code", total_code_ops, MAX_TOTAL_CODE_OPS)?;
                let func_slot_count = expect_usize(obj.get("slot_count"), "slot_count")?;
                reject_over_limit("slot_count", func_slot_count, MAX_SLOT_COUNT)?;
                let param_count = expect_usize(obj.get("param_count"), "param_count")?;
                reject_over_limit("param_count", param_count, MAX_SLOT_COUNT)?;
                let func_names = expect_string_list_limited(obj.get("names"), "names", MAX_NAMES)?;
                Ok(Function {
                    name: expect_str(obj.get("name"), "function name")?,
                    generic_params: obj
                        .get("generic_params")
                        .map(|value| expect_string_list_limited(Some(value), "generic_params", MAX_NAMES))
                        .transpose()?
                        .unwrap_or_default(),
                    param_count,
                    code: func_code,
                    slot_count: func_slot_count,
                    names: func_names,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let code = decode_code_limited(object.get("code"), "code")?;
        total_code_ops = total_code_ops
            .checked_add(code.len())
            .ok_or_else(|| TinyOneError::compile("Artifact rejected: code size overflow"))?;
        reject_over_limit("total code", total_code_ops, MAX_TOTAL_CODE_OPS)?;
        let program = Program {
            code,
            slot_count: main_slot_count,
            names: main_names,
            functions,
            strings,
            structs: raw_structs
                .iter()
                .map(|item| {
                    let obj = item
                        .as_object()
                        .ok_or_else(|| TinyOneError::compile("Struct artifact must be an object"))?;
                    let struct_fields =
                        expect_string_list_limited(obj.get("fields"), "struct fields", MAX_STRUCT_FIELDS)?;
                    Ok(StructDef {
                        name:   expect_str(obj.get("name"), "struct name")?,
                        fields: struct_fields,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            fields,
            modules: raw_modules
                .iter()
                .map(|item| {
                    let obj = item
                        .as_object()
                        .ok_or_else(|| TinyOneError::compile("Module artifact must be an object"))?;
                    let imports = optional_array_limited(obj.get("imports"), "module imports", MAX_MODULE_IMPORTS)?;
                    let exported_functions = expect_string_list_limited(
                        obj.get("exported_functions"),
                        "module function exports",
                        MAX_MODULE_EXPORTS,
                    )?;
                    let exported_structs = expect_string_list_limited(
                        obj.get("exported_structs"),
                        "module struct exports",
                        MAX_MODULE_EXPORTS,
                    )?;
                    let (capabilities, permissions) = match version {
                        JsonArtifactVersion::LegacyV1 => {
                            let capabilities = ModuleCapabilities::none();
                            (capabilities, ModulePermissions::from_capabilities(capabilities))
                        }
                        JsonArtifactVersion::V2 => {
                            let capabilities = ModuleCapabilities::from_names(&expect_string_list_limited(
                                obj.get("capabilities"),
                                "module capabilities",
                                MAX_MODULE_CAPABILITIES,
                            )?)?;
                            let permissions =
                                decode_permissions(obj.get("permissions"), "module permissions", capabilities)?;
                            (capabilities, permissions)
                        }
                    };
                    Ok(ModuleDef {
                        name: expect_str(obj.get("name"), "module name")?,
                        path: expect_str(obj.get("path"), "module path")?,
                        imports: imports
                            .iter()
                            .map(|item| {
                                let item = item
                                    .as_object()
                                    .ok_or_else(|| TinyOneError::compile("Module import must be an object"))?;
                                Ok(ModuleImportDef {
                                    alias:    expect_str(item.get("alias"), "import alias")?,
                                    path:     expect_str(item.get("path"), "import path")?,
                                    module:   expect_str(item.get("module"), "import module")?,
                                    resolved: expect_str(item.get("resolved"), "import resolved")?,
                                })
                            })
                            .collect::<Result<Vec<_>>>()?,
                        exported_functions,
                        exported_structs,
                        capabilities,
                        permissions,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            // Enum declarations do not round-trip through the JSON artifact
            // format yet; artifacts containing `Op::MakeEnum` will fail
            // verification (unknown variant index) rather than execute
            // incorrectly. Source-file compilation is unaffected.
            enum_variants: Vec::new(),
            root_capabilities,
            root_permissions,
            policy_trusted: false,
            vm_settings,
        };
        Ok(program)
    }
}

impl VerifiedProgram {
    /// Decodes an untrusted JSON artifact. Its serialized policy is retained
    /// for inspection but grants no host authority during execution.
    ///
    /// # Errors
    ///
    /// Returns an error if `data` is not a valid `TinyOne` artifact (wrong format
    /// string, unsupported version, or malformed or oversized fields), or if
    /// bytecode verification fails.
    pub fn from_artifact(data: JsonValue) -> Result<Self> {
        Self::verify(Program::decode_artifact(data)?)
    }

    /// Decodes a JSON artifact whose bytes and policy were authenticated by
    /// the embedding application. Calling this is an explicit authority
    /// decision; use [`Self::from_artifact`] for untrusted inputs.
    ///
    /// # Errors
    ///
    /// Returns an error if `data` is not a valid `TinyOne` artifact (wrong format
    /// string, unsupported version, or malformed or oversized fields), or if
    /// bytecode verification fails.
    pub fn from_trusted_artifact(data: JsonValue) -> Result<Self> {
        Self::verify(Program::decode_artifact(data)?.trust_artifact_policy())
    }
}

fn encode_code(code: &[Instr]) -> Vec<JsonValue> {
    code.iter()
        .map(|instr| json!({"op": instr.op.name(), "arg": instr.arg, "arg2": instr.arg2}))
        .collect()
}

fn encode_permissions(permissions: &ModulePermissions) -> JsonValue {
    json!({
        "filesystem_read": permissions.allows_filesystem_read(),
        "filesystem_write": permissions.allows_filesystem_write(),
        "environment_read": permissions.environment_read_allowlist(),
        "network_outbound": permissions.network_outbound(),
        "network_listen": permissions.network_listen(),
        "process_spawn": permissions.process_spawn(),
        "ffi_allowed": permissions.ffi_allowed(),
        "graphics_gpu": permissions.graphics_gpu(),
        "hardware_access": permissions.hardware_access(),
        "threads_allowed": permissions.threads_allowed(),
        "unsafe_memory_allowed": permissions.unsafe_memory_allowed(),
        "linux_pipelines_allowed": permissions.linux_pipelines_allowed(),
    })
}

fn decode_permissions(
    value: Option<&JsonValue>,
    name: &str,
    capabilities: ModuleCapabilities,
) -> Result<ModulePermissions> {
    let object = value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be an object")))?;
    let environment_read = match object.get("environment_read") {
        Some(JsonValue::Null) => None,
        Some(value) => {
            Some(expect_string_list_limited(
                Some(value),
                &format!("{name}.environment_read"),
                MAX_PERMISSION_ENVIRONMENT_VARIABLES,
            )?)
        }
        None => {
            return Err(TinyOneError::compile(format!(
                "Artifact field {name:?}.environment_read must be null or a list"
            )));
        }
    };
    ModulePermissions::from_artifact(
        capabilities,
        expect_bool(object.get("filesystem_read"), &format!("{name}.filesystem_read"))?,
        expect_bool(object.get("filesystem_write"), &format!("{name}.filesystem_write"))?,
        environment_read,
        expect_bool(object.get("network_outbound"), &format!("{name}.network_outbound"))?,
        expect_bool(object.get("network_listen"), &format!("{name}.network_listen"))?,
        expect_bool(object.get("process_spawn"), &format!("{name}.process_spawn"))?,
        expect_bool(object.get("ffi_allowed"), &format!("{name}.ffi_allowed"))?,
        expect_bool(object.get("graphics_gpu"), &format!("{name}.graphics_gpu"))?,
        expect_bool(object.get("hardware_access"), &format!("{name}.hardware_access"))?,
        expect_bool(object.get("threads_allowed"), &format!("{name}.threads_allowed"))?,
        expect_bool(object.get("unsafe_memory_allowed"), &format!("{name}.unsafe_memory_allowed"))?,
        expect_bool(object.get("linux_pipelines_allowed"), &format!("{name}.linux_pipelines_allowed"))?,
    )
}

fn decode_code_limited(value: Option<&JsonValue>, name: &str) -> Result<Vec<Instr>> {
    expect_array_limited(value, name, MAX_CODE_OPS)?
        .iter()
        .map(|item| {
            let obj = item
                .as_object()
                .ok_or_else(|| TinyOneError::compile("Instruction artifact must be an object"))?;
            let op = Op::from_name(
                obj.get("op")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| TinyOneError::compile("Instruction op must be a string"))?,
            )?;
            Ok(Instr::new(
                op,
                expect_i64(obj.get("arg"), "instruction arg")?,
                expect_i64(obj.get("arg2"), "instruction arg2")?,
            ))
        })
        .collect()
}

fn expect_array<'a>(value: Option<&'a JsonValue>, name: &str) -> Result<&'a Vec<JsonValue>> {
    value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a list")))
}

fn expect_array_limited<'a>(value: Option<&'a JsonValue>, name: &str, max: usize) -> Result<&'a Vec<JsonValue>> {
    let items = expect_array(value, name)?;
    reject_over_limit(name, items.len(), max)?;
    Ok(items)
}

fn optional_array_limited<'a>(value: Option<&'a JsonValue>, name: &str, max: usize) -> Result<&'a Vec<JsonValue>> {
    static EMPTY: Vec<JsonValue> = Vec::new();
    let items = match value {
        Some(value) => {
            value
                .as_array()
                .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a list")))?
        }
        None => &EMPTY,
    };
    reject_over_limit(name, items.len(), max)?;
    Ok(items)
}

fn expect_str(value: Option<&JsonValue>, name: &str) -> Result<String> {
    value
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a string")))
}

fn expect_usize(value: Option<&JsonValue>, name: &str) -> Result<usize> {
    let v = value
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be an integer")))?;
    usize::try_from(v)
        .map_err(|_| TinyOneError::compile(format!("Artifact field {name:?} value {v} is too large for this platform")))
}

fn expect_i64(value: Option<&JsonValue>, name: &str) -> Result<i64> {
    value
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be an integer")))
}

fn expect_bool(value: Option<&JsonValue>, name: &str) -> Result<bool> {
    value
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a boolean")))
}

fn expect_string_list_limited(value: Option<&JsonValue>, name: &str, max: usize) -> Result<Vec<String>> {
    let items = expect_array_limited(value, name, max)?;
    let mut strings = Vec::with_capacity(items.len());
    let mut bytes = 0usize;
    for item in items {
        let text = item
            .as_str()
            .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must contain strings")))?;
        bytes = bytes
            .checked_add(text.len())
            .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} is too large")))?;
        reject_over_limit(name, bytes, MAX_TEXT_BYTES)?;
        strings.push(text.to_owned());
    }
    Ok(strings)
}

fn reject_over_limit(name: &str, got: usize, max: usize) -> Result<()> {
    if got > max {
        return Err(TinyOneError::compile(format!("Artifact rejected: {name} limit {max} exceeded (got {got})")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn minimal() -> JsonValue {
        json!({
            "format": "tinyone-bytecode",
            "version": LEGACY_JSON_ARTIFACT_VERSION,
            "code": [{"op": "HALT", "arg": 0, "arg2": 0}],
            "slot_count": 0,
            "names": [],
            "functions": [],
            "strings": [],
            "structs": [],
            "fields": [],
            "modules": []
        })
    }

    fn v2_minimal() -> JsonValue {
        json!({
            "format": "tinyone-bytecode",
            "version": JSON_ARTIFACT_VERSION,
            "root_capabilities": [],
            "root_permissions": no_permissions(),
            "vm": {"max_call_depth": 1},
            "code": [{"op": "HALT", "arg": 0, "arg2": 0}],
            "slot_count": 0,
            "names": [],
            "functions": [],
            "strings": [],
            "structs": [],
            "fields": [],
            "modules": []
        })
    }

    fn no_permissions() -> JsonValue {
        json!({
            "filesystem_read": false,
            "filesystem_write": false,
            "environment_read": [],
            "network_outbound": false,
            "network_listen": false,
            "process_spawn": false,
            "ffi_allowed": false,
            "graphics_gpu": false,
            "hardware_access": false,
            "threads_allowed": false,
            "unsafe_memory_allowed": false,
            "linux_pipelines_allowed": false,
        })
    }

    fn rejects(mut artifact: JsonValue, field: &str) {
        let error = Program::from_artifact(artifact.take()).expect_err("limit must reject");
        assert!(error.to_string().contains(field), "{error}");
    }

    #[test]
    fn rejects_every_top_level_collection_limit() {
        let mut artifact = minimal();
        artifact["structs"] = JsonValue::Array(vec![json!({}); MAX_STRUCTS + 1]);
        rejects(artifact, "structs limit");

        let mut artifact = minimal();
        artifact["fields"] = JsonValue::Array(vec![json!(""); MAX_FIELDS + 1]);
        rejects(artifact, "fields limit");

        let mut artifact = minimal();
        artifact["modules"] = JsonValue::Array(vec![json!({}); MAX_MODULES + 1]);
        rejects(artifact, "modules limit");

        let mut artifact = minimal();
        artifact["names"] = JsonValue::Array(vec![json!(""); MAX_NAMES + 1]);
        rejects(artifact, "names limit");
    }

    #[test]
    fn rejects_nested_struct_and_module_limits_before_collecting() {
        let mut artifact = minimal();
        artifact["structs"] = json!([{
            "name": "TooWide",
            "fields": vec![""; MAX_STRUCT_FIELDS + 1]
        }]);
        rejects(artifact, "struct fields limit");

        let mut artifact = minimal();
        artifact["modules"] = json!([{
            "name": "m", "path": "m",
            "imports": vec![{}; MAX_MODULE_IMPORTS + 1],
            "exported_functions": [], "exported_structs": []
        }]);
        rejects(artifact, "module imports limit");

        let mut artifact = minimal();
        artifact["modules"] = json!([{
            "name": "m", "path": "m", "imports": [],
            "exported_functions": vec![""; MAX_MODULE_EXPORTS + 1],
            "exported_structs": []
        }]);
        rejects(artifact, "module function exports limit");
    }

    #[test]
    fn rejects_nested_function_and_total_code_limits_before_verification() {
        let mut artifact = minimal();
        artifact["functions"] = json!([{
            "name": "too_long",
            "param_count": 0,
            "code": vec![json!({"op": "HALT", "arg": 0, "arg2": 0}); MAX_CODE_OPS + 1],
            "slot_count": 0,
            "names": []
        }]);
        rejects(artifact, "function code limit");

        let mut artifact = minimal();
        artifact["functions"] = json!([{
            "name": "too_many_params",
            "param_count": MAX_SLOT_COUNT + 1,
            "code": [{"op": "HALT", "arg": 0, "arg2": 0}],
            "slot_count": 0,
            "names": []
        }]);
        rejects(artifact, "param_count limit");

        let function = json!({
            "name": "f",
            "param_count": 0,
            "code": vec![json!({"op": "HALT", "arg": 0, "arg2": 0}); MAX_CODE_OPS],
            "slot_count": 0,
            "names": []
        });
        let mut artifact = minimal();
        artifact["functions"] = JsonValue::Array(vec![function; 5]);
        rejects(artifact, "total code limit");
    }

    #[test]
    fn rejects_text_budget_overflow() {
        let mut artifact = minimal();
        artifact["strings"] = json!(["x".repeat(MAX_TEXT_BYTES + 1)]);
        rejects(artifact, "strings limit");
    }

    #[test]
    fn policy_bearing_artifacts_use_v2_and_round_trip_restrictions() {
        let artifact = v2_minimal();
        let decoded = Program::from_artifact(artifact.clone()).expect("v2 artifact decodes");
        // assert!(decoded.root_capabilities().is_empty());
        assert_eq!(decoded.root_capabilities(), [] as [std::string::String; 0]);
        assert_eq!(decoded.max_call_depth(), 1);

        let emitted = decoded.to_artifact();
        assert_eq!(emitted["version"], JSON_ARTIFACT_VERSION);
        assert_eq!(emitted["root_capabilities"], json!([]));
        assert_eq!(emitted["root_permissions"], no_permissions());
        assert_eq!(emitted["vm"]["max_call_depth"], 1);
    }

    #[test]
    fn v2_requires_complete_policy_fields() {
        let mut artifact = minimal();
        artifact["version"] = json!(JSON_ARTIFACT_VERSION);
        rejects(artifact, "root_capabilities");

        let mut artifact = v2_minimal();
        artifact.as_object_mut().expect("object").remove("vm");
        rejects(artifact, "vm");

        let mut artifact = v2_minimal();
        artifact["modules"] = json!([{
            "name": "m",
            "path": "m",
            "imports": [],
            "exported_functions": [],
            "exported_structs": [],
            "capabilities": []
        }]);
        rejects(artifact, "module permissions");

        let mut artifact = v2_minimal();
        artifact.as_object_mut().expect("object").remove("root_permissions");
        rejects(artifact, "root_permissions");

        let mut artifact = v2_minimal();
        artifact["root_permissions"]["filesystem_read"] = json!(true);
        rejects(artifact, "detailed permissions exceed");
    }

    #[test]
    fn legacy_v1_uses_historical_policy_even_with_policy_lookalikes() {
        let mut artifact = minimal();
        artifact["root_capabilities"] = json!([]);
        artifact["vm"] = json!({"max_call_depth": 1});
        artifact["modules"] = json!([{
            "name": "m",
            "path": "m",
            "imports": [],
            "exported_functions": [],
            "exported_structs": [],
            "capabilities": ["filesystem"]
        }]);

        let decoded = Program::from_artifact(artifact).expect("legacy artifact decodes");
        assert_eq!(decoded.root_capabilities(), ModuleCapabilities::all().names());
        assert_eq!(decoded.max_call_depth(), crate::MAX_CALL_DEPTH);
        assert_eq!(decoded.modules()[0].capabilities(), [] as [std::string::String; 0]);
    }

    #[test]
    fn v2_round_trips_fine_grained_module_permissions() {
        let mut artifact = v2_minimal();
        artifact["modules"] = json!([{
            "name": "m",
            "path": "m",
            "imports": [],
            "exported_functions": [],
            "exported_structs": [],
            "capabilities": ["filesystem", "environment"],
            "permissions": {
                "filesystem_read": true,
                "filesystem_write": false,
                "environment_read": ["HTTP_PROXY"],
                "network_outbound": false,
                "network_listen": false,
                "process_spawn": false,
                "ffi_allowed": false,
                "graphics_gpu": false,
                "hardware_access": false,
                "threads_allowed": false,
                "unsafe_memory_allowed": false,
                "linux_pipelines_allowed": false
            }
        }]);

        let decoded = Program::from_artifact(artifact).expect("v2 artifact decodes");
        let module = &decoded.modules()[0];
        assert_eq!(module.filesystem_permissions(), (true, false));
        assert_eq!(module.environment_read_allowlist(), Some(["HTTP_PROXY".to_string()].as_slice()));
        let encoded = decoded.to_artifact();
        assert_eq!(encoded["modules"][0]["permissions"]["filesystem_write"], false);
        assert_eq!(encoded["modules"][0]["permissions"]["environment_read"], json!(["HTTP_PROXY"]));
    }
}
