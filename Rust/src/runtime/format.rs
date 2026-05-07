use std::collections::HashSet;
use std::io::Write;

use crate::{HeapData, Result, TinyOneError, TinyRuntimeContext, Value};

pub(crate) fn runtime_format(context: &TinyRuntimeContext, value: &Value) -> Result<String> {
    runtime_format_inner(context, value, &mut HashSet::new())
}

fn runtime_format_inner(
    context: &TinyRuntimeContext,
    value: &Value,
    seen: &mut HashSet<usize>,
) -> Result<String> {
    match value {
        Value::Int(value) => Ok(value.to_string()),
        Value::Pointer(pointer) => {
            let suffix = if pointer.cast.is_empty() {
                String::new()
            } else {
                format!(":{}", pointer.cast)
            };
            if pointer.kind == "null" && pointer.address == 0 {
                Ok("null".to_string())
            } else if pointer.kind == "array" {
                Ok(format!(
                    "ptr(array@{}[{}]{suffix})",
                    pointer.address, pointer.index
                ))
            } else if pointer.kind == "buffer" {
                Ok(format!(
                    "ptr(buffer@{}+{}{suffix})",
                    pointer.address, pointer.index
                ))
            } else if pointer.kind == "field" {
                Ok(format!(
                    "ptr(field@{}.{}{suffix})",
                    pointer.address, pointer.field
                ))
            } else {
                Ok(format!("ptr({}@{}{suffix})", pointer.kind, pointer.address))
            }
        }
        Value::Heap(reference) => {
            let object = context.heap.get(value)?.clone();
            if seen.contains(&reference.address) {
                return Ok(format!("&{}<cycle>", reference.address));
            }
            seen.insert(reference.address);
            let rendered = match object.data {
                HeapData::String(text) => Ok(text),
                HeapData::Array(values) => {
                    let mut parts = Vec::with_capacity(values.len());
                    for item in &values {
                        parts.push(runtime_format_inner(context, item, seen)?);
                    }
                    Ok(format!("[{}]", parts.join(", ")))
                }
                HeapData::Buffer(data) => Ok(format!(
                    "buffer[{}]",
                    data.iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
                HeapData::Struct(fields) => {
                    let mut parts = Vec::with_capacity(fields.len());
                    for (name, value) in &fields {
                        parts.push(format!(
                            "{name}: {}",
                            runtime_format_inner(context, value, seen)?
                        ));
                    }
                    Ok(format!("{}{{{}}}", object.type_name, parts.join(", ")))
                }
                HeapData::Cell(value) => Ok(format!(
                    "&{}({})",
                    reference.address,
                    runtime_format_inner(context, &value, seen)?
                )),
            };
            seen.remove(&reference.address);
            rendered
        }
    }
}

pub(crate) fn runtime_print(
    context: &TinyRuntimeContext,
    stdout: &mut dyn Write,
    value: &Value,
) -> Result<()> {
    writeln!(stdout, "{}", runtime_format(context, value)?)
        .map_err(|error| TinyOneError::runtime(format!("Write error: {error}")))
}
