use std::collections::HashSet;
use std::io::Write;

use crate::{HeapData, Result, TinyOneError, TinyRuntimeContext, Value};

pub(crate) fn runtime_format(context: &TinyRuntimeContext, value: &Value) -> Result<String> {
    runtime_format_inner(context, value, &mut HashSet::new())
}

fn runtime_format_inner(context: &TinyRuntimeContext, value: &Value, seen: &mut HashSet<usize>) -> Result<String> {
    match value {
        Value::I64(value) => Ok(value.to_string()),
        Value::U8(value) => Ok(value.to_string()),
        Value::U16(value) => Ok(value.to_string()),
        Value::U32(value) => Ok(value.to_string()),
        Value::I8(value) => Ok(value.to_string()),
        Value::I16(value) => Ok(value.to_string()),
        Value::I32(value) => Ok(value.to_string()),
        Value::U64(value) => Ok(value.to_string()),
        Value::Bf16(bits) => Ok(format!("bf16({bits})")),
        Value::Float { bits, .. } => Ok(bits.to_string()),
        Value::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        Value::Unit => Ok("unit".to_string()),
        Value::Null => Ok("null".to_string()),
        Value::Function(idx) => Ok(format!("fn#{idx}")),
        Value::Reference(r) => Ok(format!("ref@{}", r.address)),
        Value::Phantom => Ok("phantom".to_string()),
        Value::Zst(_) => Ok("zst".to_string()),
        Value::Unsafe => Ok("unsafe".to_string()),
        Value::Pointer(pointer) => {
            let suffix = if pointer.cast.is_empty() {
                String::new()
            } else {
                format!(":{}", pointer.cast)
            };
            Ok(match pointer.kind.as_str() {
                "null" if pointer.address == 0 => "null".to_string(),
                "array" => format!("ptr(array@{}[{}]{suffix})", pointer.address, pointer.index),
                "buffer" => format!("ptr(buffer@{}+{}{suffix})", pointer.address, pointer.index),
                "field" => format!("ptr(field@{}.{}{suffix})", pointer.address, pointer.field),
                kind => format!("ptr({kind}@{}{suffix})", pointer.address),
            })
        }
        Value::Heap(reference) => {
            let object = context.heap().get(value)?.clone();
            if seen.contains(&reference.address) {
                return Ok(format!("&{}<cycle>", reference.address));
            }
            seen.insert(reference.address);
            let rendered = match object.data {
                HeapData::String(text) => Ok(text),
                HeapData::Array(values) => {
                    let parts = values
                        .iter()
                        .map(|item| runtime_format_inner(context, item, seen))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("[{}]", parts.join(", ")))
                }
                HeapData::Buffer(data) => {
                    Ok(format!(
                        "buffer[{}]",
                        data.iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ))
                }
                HeapData::Struct(fields) => {
                    let parts = fields
                        .iter()
                        .map(|(name, value)| {
                            runtime_format_inner(context, value, seen).map(|rendered| format!("{name}: {rendered}"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("{}{{{}}}", object.type_name, parts.join(", ")))
                }
                HeapData::Cell(value) => {
                    Ok(format!("&{}({})", reference.address, runtime_format_inner(context, &value, seen)?))
                }
                HeapData::Map(entries) => {
                    let parts = entries
                        .iter()
                        .map(|(key, value)| {
                            let key = runtime_format_inner(context, key, seen)?;
                            let value = runtime_format_inner(context, value, seen)?;
                            Ok(format!("{key}: {value}"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("map{{{}}}", parts.join(", ")))
                }
                HeapData::Mutex(_) => Ok(format!("mutex@{}", reference.address)),
                HeapData::Atomic(a) => Ok(format!("atomic({})", a.load(std::sync::atomic::Ordering::Relaxed))),
                HeapData::Thread(_) => Ok(format!("thread@{}", reference.address)),
                HeapData::Char(scalar) => {
                    Ok(format!("{}", char::from_u32(scalar).unwrap_or(char::REPLACEMENT_CHARACTER)))
                }
                HeapData::CharBuffer(chars) => {
                    Ok(chars
                        .iter()
                        .map(|&c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
                        .collect::<String>())
                }
                HeapData::Vec(values) => {
                    let parts = values
                        .iter()
                        .map(|item| runtime_format_inner(context, item, seen))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("[{}]", parts.join(", ")))
                }
                HeapData::Record(fields) => {
                    let parts = fields
                        .iter()
                        .map(|(name, value)| {
                            runtime_format_inner(context, value, seen).map(|rendered| format!("{name}: {rendered}"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("{{{}}}", parts.join(", ")))
                }
                HeapData::Dictionary(entries) => {
                    let parts = entries
                        .iter()
                        .map(|(key, value)| {
                            let key = runtime_format_inner(context, key, seen)?;
                            let value = runtime_format_inner(context, value, seen)?;
                            Ok(format!("{key}: {value}"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("dict{{{}}}", parts.join(", ")))
                }
                HeapData::Box(inner) => Ok(format!("box({})", runtime_format_inner(context, &*inner, seen)?)),
                HeapData::Alloc { data, .. } => Ok(format!("alloc[{}b]", data.len())),
                HeapData::Closure { function_id, captures } => {
                    Ok(format!("closure(fn#{}, {} captures)", function_id, captures.len()))
                }
                HeapData::Sum { tag, payload } => {
                    let payload_str = if let Some(p) = payload {
                        runtime_format_inner(context, &*p, seen)?
                    } else {
                        "()".to_string()
                    };
                    Ok(format!("sum({tag}, {payload_str})"))
                }
                HeapData::Enum { variant } => Ok(format!("enum({variant})")),
                HeapData::TaggedUnion { tag, payload } => {
                    Ok(format!("union({tag}, {})", runtime_format_inner(context, &*payload, seen)?))
                }
                HeapData::Result { is_ok, value } => {
                    let inner = runtime_format_inner(context, &*value, seen)?;
                    if is_ok {
                        Ok(format!("Ok({inner})"))
                    } else {
                        Ok(format!("Err({inner})"))
                    }
                }
                HeapData::Option { value } => {
                    match value {
                        Some(inner) => Ok(format!("Some({})", runtime_format_inner(context, &*inner, seen)?)),
                        None => Ok("None".to_string()),
                    }
                }
                HeapData::Dyn {
                    type_id,
                    vtable_id,
                    value,
                } => {
                    Ok(format!(
                        "dyn(type={type_id}, vtable={vtable_id}, {})",
                        runtime_format_inner(context, &*value, seen)?
                    ))
                }
                HeapData::FileDescriptor(fd) => Ok(format!("fd({fd})")),
            };
            seen.remove(&reference.address);
            rendered
        }
    }
}

pub(crate) fn runtime_print(context: &TinyRuntimeContext, stdout: &mut dyn Write, value: &Value) -> Result<()> {
    writeln!(stdout, "{}", runtime_format(context, value)?)
        .map_err(|error| TinyOneError::runtime(format!("Write error: {error}")))
}
