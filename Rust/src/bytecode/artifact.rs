use serde_json::{Value as JsonValue, json};

use crate::{
    BytecodeVerifier, Function, Instr, ModuleDef, ModuleImportDef, Op, Program, Result, StructDef,
    TinyOneError,
};

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
        let functions = expect_array(object.get("functions"), "functions")?
            .iter()
            .map(|item| {
                let obj = item
                    .as_object()
                    .ok_or_else(|| TinyOneError::compile("Function artifact must be an object"))?;
                Ok(Function {
                    name: expect_str(obj.get("name"), "function name")?,
                    param_count: expect_usize(obj.get("param_count"), "param_count")?,
                    code: decode_code(obj.get("code"))?,
                    slot_count: expect_usize(obj.get("slot_count"), "slot_count")?,
                    names: expect_string_list(obj.get("names"), "names")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let program = Program {
            code: decode_code(object.get("code"))?,
            slot_count: expect_usize(object.get("slot_count"), "slot_count")?,
            names: expect_string_list(object.get("names"), "names")?,
            functions,
            strings: expect_string_list(object.get("strings"), "strings")?,
            structs: expect_array(object.get("structs"), "structs")?
                .iter()
                .map(|item| {
                    let obj = item.as_object().ok_or_else(|| {
                        TinyOneError::compile("Struct artifact must be an object")
                    })?;
                    Ok(StructDef {
                        name: expect_str(obj.get("name"), "struct name")?,
                        fields: expect_string_list(obj.get("fields"), "struct fields")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            fields: expect_string_list(object.get("fields"), "fields")?,
            modules: optional_array(object.get("modules"), "modules")?
                .iter()
                .map(|item| {
                    let obj = item.as_object().ok_or_else(|| {
                        TinyOneError::compile("Module artifact must be an object")
                    })?;
                    Ok(ModuleDef {
                        name: expect_str(obj.get("name"), "module name")?,
                        path: expect_str(obj.get("path"), "module path")?,
                        imports: optional_array(obj.get("imports"), "module imports")?
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
                        exported_functions: expect_string_list(
                            obj.get("exported_functions"),
                            "module function exports",
                        )?,
                        exported_structs: expect_string_list(
                            obj.get("exported_structs"),
                            "module struct exports",
                        )?,
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

fn decode_code(value: Option<&JsonValue>) -> Result<Vec<Instr>> {
    expect_array(value, "code")?
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
                obj.get("arg").and_then(JsonValue::as_i64).unwrap_or(0),
                obj.get("arg2").and_then(JsonValue::as_i64).unwrap_or(0),
            ))
        })
        .collect()
}

fn expect_array<'a>(value: Option<&'a JsonValue>, name: &str) -> Result<&'a Vec<JsonValue>> {
    value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a list")))
}

fn optional_array<'a>(value: Option<&'a JsonValue>, name: &str) -> Result<&'a Vec<JsonValue>> {
    static EMPTY: Vec<JsonValue> = Vec::new();
    match value {
        Some(value) => value.as_array().ok_or_else(|| {
            TinyOneError::compile(format!("Artifact field {name:?} must be a list"))
        }),
        None => Ok(&EMPTY),
    }
}

fn expect_str(value: Option<&JsonValue>, name: &str) -> Result<String> {
    value
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a string")))
}

fn expect_usize(value: Option<&JsonValue>, name: &str) -> Result<usize> {
    value
        .and_then(JsonValue::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be an integer")))
}

fn expect_string_list(value: Option<&JsonValue>, name: &str) -> Result<Vec<String>> {
    expect_array(value, name)?
        .iter()
        .map(|item| {
            item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                TinyOneError::compile(format!("Artifact field {name:?} must contain strings"))
            })
        })
        .collect()
}
