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
        // `{:?}` (not `{}`) so whole-number floats print with a decimal
        // point (`4.0`, not `4`) — Rust's `Display` for `f64` drops it.
        Value::Float { bits, .. } => Ok(format!("{bits:?}")),
        Value::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        Value::Unit => Ok("unit".to_string()),
        Value::Null => Ok("null".to_string()),
        Value::Function(idx) => Ok(format!("fn#{idx}")),
        Value::Reference(r) => Ok(format!("ref@{}", r.address)),
        Value::Phantom => Ok("phantom".to_string()),
        Value::Zst(_) => Ok("zst".to_string()),
        Value::Unsafe => Ok("unsafe".to_string()),
        Value::Pointer(pointer) => {
            let suffix = if pointer.cast == crate::runtime::value::CastKind::None {
                String::new()
            } else {
                format!(":{}", pointer.cast.as_str())
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
            if seen.contains(&reference.address) {
                return Ok(format!("&{}<cycle>", reference.address));
            }
            seen.insert(reference.address);

            // `HeapData` isn't `Clone` (its `String`/`Buffer`/`CharBuffer`
            // variants own a real, capacity-bounded `RallocBytes` that can
            // only be duplicated via a fallible allocation — see
            // `runtime/heap.rs`). So instead of cloning the whole object up
            // front, each arm below either renders its result directly while
            // the heap lock is still held (leaf kinds), or clones just the
            // cheap `Value`-based payload it needs and drops the lock before
            // recursing (container kinds) — recursing while still holding
            // the heap's mutex would deadlock.
            let heap = context.heap();
            let object = heap.get(value)?;
            let rendered = match &object.data {
                HeapData::String(text) => Ok(crate::runtime::heap::heap_str(text)?.to_owned()),
                HeapData::Array(values) => {
                    let values = crate::runtime::heap::decode_array_values(values);
                    drop(heap);
                    let parts = values
                        .iter()
                        .map(|item| runtime_format_inner(context, item, seen))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("[{}]", parts.join(", ")))
                }
                HeapData::Buffer(data) => {
                    Ok(format!(
                        "buffer[{}]",
                        data.as_slice()
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ))
                }
                HeapData::Struct(record) => {
                    let fields = record.fields();
                    let type_name = object.type_name.clone();
                    drop(heap);
                    let parts = fields
                        .iter()
                        .map(|(name, value)| {
                            runtime_format_inner(context, value, seen).map(|rendered| format!("{name}: {rendered}"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("{}{{{}}}", type_name, parts.join(", ")))
                }
                HeapData::Cell(bytes) => {
                    let value = crate::runtime::value_codec::decode_value(bytes.as_slice().try_into().unwrap());
                    drop(heap);
                    Ok(format!("&{}({})", reference.address, runtime_format_inner(context, &value, seen)?))
                }
                HeapData::Map(entries) => {
                    let entries = crate::runtime::heap::decode_map_entries(entries);
                    drop(heap);
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
                    Ok(format!("{}", char::from_u32(*scalar).unwrap_or(char::REPLACEMENT_CHARACTER)))
                }
                HeapData::CharBuffer(chars) => {
                    Ok(crate::runtime::heap::unpack_char_buffer(chars.as_slice())
                        .into_iter()
                        .map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
                        .collect::<String>())
                }
                HeapData::Vec(values) => {
                    let values = crate::runtime::heap::decode_array_values(values);
                    drop(heap);
                    let parts = values
                        .iter()
                        .map(|item| runtime_format_inner(context, item, seen))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("[{}]", parts.join(", ")))
                }
                HeapData::Record(record) => {
                    let fields = record.fields();
                    drop(heap);
                    let parts = fields
                        .iter()
                        .map(|(name, value)| {
                            runtime_format_inner(context, value, seen).map(|rendered| format!("{name}: {rendered}"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(format!("{{{}}}", parts.join(", ")))
                }
                HeapData::Dictionary(entries) => {
                    let entries = crate::runtime::heap::decode_map_entries(entries);
                    drop(heap);
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
                HeapData::Box(inner) => {
                    let inner = crate::runtime::heap::decode_value_slot(inner);
                    drop(heap);
                    Ok(format!("box({})", runtime_format_inner(context, &inner, seen)?))
                }
                HeapData::Alloc { data, .. } => Ok(format!("alloc[{}b]", data.len())),
                HeapData::Closure { function_id, captures } => {
                    Ok(format!("closure(fn#{}, {} captures)", function_id, captures.len()))
                }
                HeapData::Sum { tag, payload } => {
                    let tag = *tag;
                    let payload = payload.as_ref().map(crate::runtime::heap::decode_value_slot);
                    drop(heap);
                    let payload_str = if let Some(p) = payload {
                        runtime_format_inner(context, &p, seen)?
                    } else {
                        "()".to_string()
                    };
                    Ok(format!("sum({tag}, {payload_str})"))
                }
                HeapData::Enum(record) => {
                    let variant = record.variant().to_owned();
                    let fields = record.fields();
                    let type_name = object.type_name.clone();
                    drop(heap);
                    if fields.is_empty() {
                        Ok(format!("{type_name}.{variant}"))
                    } else {
                        let parts = fields
                            .iter()
                            .map(|(name, value)| {
                                runtime_format_inner(context, value, seen).map(|rendered| format!("{name}: {rendered}"))
                            })
                            .collect::<Result<Vec<_>>>()?;
                        Ok(format!("{}.{}{{{}}}", type_name, variant, parts.join(", ")))
                    }
                }
                HeapData::TaggedUnion { tag, payload } => {
                    let tag = *tag;
                    let payload = crate::runtime::heap::decode_value_slot(payload);
                    drop(heap);
                    Ok(format!("union({tag}, {})", runtime_format_inner(context, &payload, seen)?))
                }
                HeapData::Result { is_ok, value } => {
                    let is_ok = *is_ok;
                    let value = crate::runtime::heap::decode_value_slot(value);
                    drop(heap);
                    let inner = runtime_format_inner(context, &value, seen)?;
                    if is_ok {
                        Ok(format!("Ok({inner})"))
                    } else {
                        Ok(format!("Err({inner})"))
                    }
                }
                HeapData::Option { value } => {
                    let value = value.as_ref().map(crate::runtime::heap::decode_value_slot);
                    drop(heap);
                    match value {
                        Some(inner) => Ok(format!("Some({})", runtime_format_inner(context, &inner, seen)?)),
                        None => Ok("None".to_string()),
                    }
                }
                HeapData::Dyn {
                    type_id,
                    vtable_id,
                    value,
                } => {
                    let type_id = *type_id;
                    let vtable_id = *vtable_id;
                    let value = crate::runtime::heap::decode_value_slot(value);
                    drop(heap);
                    Ok(format!(
                        "dyn(type={type_id}, vtable={vtable_id}, {})",
                        runtime_format_inner(context, &value, seen)?
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
