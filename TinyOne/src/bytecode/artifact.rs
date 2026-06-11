use serde_json::{Value as JsonValue, json};

use crate::{
    BytecodeVerifier, Function, Instr, ModuleDef, ModuleImportDef, Op, Program, Result, StructDef,
    TinyOneError,
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
const MAX_STRUCT_FIELDS: usize = 256;
#[allow(dead_code)]
const MAX_ENUM_VARIANTS: usize = 65_536;
const MAX_NAMES: usize = 65_536;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

impl Program {
    pub fn to_artifact(&self) -> JsonValue {
        json!({
            "format": "tinyone-bytecode",
            "version": 1,
            "code": encode_code(&self.code),
            "slot_count": self.slot_count,
            "names": self.names,
            "functions": self.functions.iter().map(|function| json!({
                "name": function.name,
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
            })).collect::<Vec<_>>(),
        })
    }

    pub fn from_artifact(data: JsonValue) -> Result<Self> {
        let object = data
            .as_object()
            .ok_or_else(|| TinyOneError::compile("Artifact must be a JSON object"))?;
        if object.get("format").and_then(JsonValue::as_str) != Some("tinyone-bytecode")
            || object.get("version").and_then(JsonValue::as_i64) != Some(1)
        {
            return Err(TinyOneError::compile("Unsupported TinyOne artifact format"));
        }
        let raw_functions =
            expect_array_limited(object.get("functions"), "functions", MAX_FUNCTIONS)?;
        let main_slot_count = expect_usize(object.get("slot_count"), "slot_count")?;
        reject_over_limit("slot_count", main_slot_count, MAX_SLOT_COUNT)?;
        let main_names = expect_string_list_limited(object.get("names"), "names", MAX_NAMES)?;
        let strings = expect_string_list_limited(object.get("strings"), "strings", MAX_STRINGS)?;
        let fields = expect_string_list_limited(object.get("fields"), "fields", MAX_FIELDS)?;
        let raw_structs = expect_array_limited(object.get("structs"), "structs", MAX_STRUCTS)?;
        let raw_modules = optional_array_limited(object.get("modules"), "modules", MAX_MODULES)?;
        let mut total_code_ops = 0usize;
        let functions = raw_functions
            .iter()
            .map(|item| {
                let obj = item
                    .as_object()
                    .ok_or_else(|| TinyOneError::compile("Function artifact must be an object"))?;
                let func_code = decode_code_limited(obj.get("code"), "function code")?;
                total_code_ops = total_code_ops.checked_add(func_code.len()).ok_or_else(|| {
                    TinyOneError::compile("Artifact rejected: code size overflow")
                })?;
                reject_over_limit("total code", total_code_ops, MAX_TOTAL_CODE_OPS)?;
                let func_slot_count = expect_usize(obj.get("slot_count"), "slot_count")?;
                reject_over_limit("slot_count", func_slot_count, MAX_SLOT_COUNT)?;
                let param_count = expect_usize(obj.get("param_count"), "param_count")?;
                reject_over_limit("param_count", param_count, MAX_SLOT_COUNT)?;
                let func_names = expect_string_list_limited(obj.get("names"), "names", MAX_NAMES)?;
                Ok(Function {
                    name: expect_str(obj.get("name"), "function name")?,
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
                    let obj = item.as_object().ok_or_else(|| {
                        TinyOneError::compile("Struct artifact must be an object")
                    })?;
                    let struct_fields = expect_string_list_limited(
                        obj.get("fields"),
                        "struct fields",
                        MAX_STRUCT_FIELDS,
                    )?;
                    Ok(StructDef {
                        name: expect_str(obj.get("name"), "struct name")?,
                        fields: struct_fields,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            fields,
            modules: raw_modules
                .iter()
                .map(|item| {
                    let obj = item.as_object().ok_or_else(|| {
                        TinyOneError::compile("Module artifact must be an object")
                    })?;
                    let imports = optional_array_limited(
                        obj.get("imports"),
                        "module imports",
                        MAX_MODULE_IMPORTS,
                    )?;
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
                    Ok(ModuleDef {
                        name: expect_str(obj.get("name"), "module name")?,
                        path: expect_str(obj.get("path"), "module path")?,
                        imports: imports
                            .iter()
                            .map(|item| {
                                let item = item.as_object().ok_or_else(|| {
                                    TinyOneError::compile("Module import must be an object")
                                })?;
                                Ok(ModuleImportDef {
                                    alias: expect_str(item.get("alias"), "import alias")?,
                                    path: expect_str(item.get("path"), "import path")?,
                                    module: expect_str(item.get("module"), "import module")?,
                                    resolved: expect_str(item.get("resolved"), "import resolved")?,
                                })
                            })
                            .collect::<Result<Vec<_>>>()?,
                        exported_functions,
                        exported_structs,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };
        BytecodeVerifier::verify(&program)?;
        Ok(program)
    }
}

fn encode_code(code: &[Instr]) -> Vec<JsonValue> {
    code.iter()
        .map(|instr| json!({"op": instr.op.name(), "arg": instr.arg, "arg2": instr.arg2}))
        .collect()
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

fn expect_array_limited<'a>(
    value: Option<&'a JsonValue>,
    name: &str,
    max: usize,
) -> Result<&'a Vec<JsonValue>> {
    let items = expect_array(value, name)?;
    reject_over_limit(name, items.len(), max)?;
    Ok(items)
}

fn optional_array_limited<'a>(
    value: Option<&'a JsonValue>,
    name: &str,
    max: usize,
) -> Result<&'a Vec<JsonValue>> {
    static EMPTY: Vec<JsonValue> = Vec::new();
    let items = match value {
        Some(value) => value.as_array().ok_or_else(|| {
            TinyOneError::compile(format!("Artifact field {name:?} must be a list"))
        })?,
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
    let v = value.and_then(JsonValue::as_u64).ok_or_else(|| {
        TinyOneError::compile(format!("Artifact field {name:?} must be an integer"))
    })?;
    usize::try_from(v).map_err(|_| {
        TinyOneError::compile(format!(
            "Artifact field {name:?} value {v} is too large for this platform"
        ))
    })
}

fn expect_i64(value: Option<&JsonValue>, name: &str) -> Result<i64> {
    value
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be an integer")))
}

fn expect_string_list_limited(
    value: Option<&JsonValue>,
    name: &str,
    max: usize,
) -> Result<Vec<String>> {
    let items = expect_array_limited(value, name, max)?;
    let mut strings = Vec::with_capacity(items.len());
    let mut bytes = 0usize;
    for item in items {
        let text = item.as_str().ok_or_else(|| {
            TinyOneError::compile(format!("Artifact field {name:?} must contain strings"))
        })?;
        bytes = bytes.checked_add(text.len()).ok_or_else(|| {
            TinyOneError::compile(format!("Artifact field {name:?} is too large"))
        })?;
        reject_over_limit(name, bytes, MAX_TEXT_BYTES)?;
        strings.push(text.to_owned());
    }
    Ok(strings)
}

fn reject_over_limit(name: &str, got: usize, max: usize) -> Result<()> {
    if got > max {
        return Err(TinyOneError::compile(format!(
            "Artifact rejected: {name} limit {max} exceeded (got {got})"
        )));
    }
    Ok(())
}
