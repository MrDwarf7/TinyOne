use crate::runtime::value::{CastKind, PointerKind};
use crate::{
    HeapData, MAX_BUFFER_BYTES, RawPointer, Result, TinyOneError, TinyRuntimeContext, Value,
    checked_bounded_len, checked_byte_range, checked_collection_index, checked_non_negative_usize,
    expect_int, expect_string,
};

pub(crate) fn expect_pointer(value: &Value, operation: &str) -> Result<RawPointer> {
    match value {
        Value::Pointer(pointer) => Ok(pointer.clone()),
        Value::Null => Ok(RawPointer::new(
            0,
            PointerKind::Null,
            0,
            "",
            0,
            CastKind::None,
        )),
        _ => Err(TinyOneError::runtime(format!(
            "{operation} expects a raw pointer"
        ))),
    }
}

pub(crate) fn validate_pointer_base(
    context: &TinyRuntimeContext,
    pointer: &RawPointer,
    operation: &str,
) -> Result<()> {
    if pointer.kind == PointerKind::Null && pointer.address == 0 {
        return Ok(());
    }
    match pointer.kind {
        PointerKind::Object | PointerKind::Array | PointerKind::Buffer | PointerKind::Field => {
            context
                .heap()
                .get_address(pointer.address, pointer.generation)?;
            Ok(())
        }
        PointerKind::Null => Err(TinyOneError::runtime(format!(
            "{operation} got unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

fn pointer_identity(pointer: &RawPointer) -> (usize, u64, PointerKind, i64, String) {
    if pointer.kind == PointerKind::Null && pointer.address == 0 {
        return (0, 0, PointerKind::Null, 0, String::new());
    }
    (
        pointer.address,
        pointer.generation,
        pointer.kind,
        pointer.index,
        pointer.field.clone(),
    )
}

pub(crate) fn runtime_make_pointer(context: &TinyRuntimeContext, args: &[Value]) -> Result<Value> {
    if args.len() == 1 {
        match &args[0] {
            Value::Pointer(pointer) => return Ok(Value::Pointer(pointer.clone())),
            Value::Heap(reference) => {
                context.heap().get(&args[0])?;
                return Ok(Value::Pointer(RawPointer::new(
                    reference.address,
                    PointerKind::Object,
                    0,
                    "",
                    reference.generation,
                    CastKind::None,
                )));
            }
            _ => {
                return Err(TinyOneError::runtime(
                    "ptr() expects a heap value or pointer",
                ));
            }
        }
    }
    let target = &args[0];
    let index = expect_int(&args[1], "ptr index")?;
    let Value::Heap(reference) = target else {
        return Err(TinyOneError::runtime(
            "ptr(value, index) expects an array or buffer heap value",
        ));
    };
    // `HeapObject::kind()`'s string space (23+ heap-object kinds) and
    // `PointerKind`'s (5 pointer-location kinds) overlap only at these two
    // literals — translate explicitly rather than reusing one enum for both.
    let kind = context.heap().get(target)?.kind();
    match kind {
        "array" => Ok(Value::Pointer(RawPointer::new(
            reference.address,
            PointerKind::Array,
            index,
            "",
            reference.generation,
            CastKind::None,
        ))),
        "buffer" => Ok(Value::Pointer(RawPointer::new(
            reference.address,
            PointerKind::Buffer,
            index,
            "",
            reference.generation,
            CastKind::None,
        ))),
        _ => Err(TinyOneError::runtime(
            "ptr(value, index) expects an array or buffer heap value",
        )),
    }
}

pub(crate) fn runtime_make_field_pointer(
    context: &TinyRuntimeContext,
    target: &Value,
    field_value: &Value,
) -> Result<Value> {
    let Value::Heap(reference) = target else {
        return Err(TinyOneError::runtime(
            "fieldptr() expects a struct heap value",
        ));
    };
    // Evaluate the field-name argument before taking the heap lock — it may
    // itself need to read the heap (e.g. a string heap value), and the heap
    // mutex isn't reentrant.
    let field = expect_string(context, field_value, "fieldptr")?;
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Struct(record) = &object.data else {
        return Err(TinyOneError::runtime(
            "fieldptr() expects a struct heap value",
        ));
    };
    if !record.contains(&field) {
        return Err(TinyOneError::runtime(format!(
            "Unknown field {field:?} on struct {:?}",
            object.type_name
        )));
    }
    Ok(Value::Pointer(RawPointer::new(
        reference.address,
        PointerKind::Field,
        0,
        field,
        reference.generation,
        CastKind::None,
    )))
}

pub(crate) fn runtime_pointer_address(
    context: &TinyRuntimeContext,
    value: &Value,
) -> Result<Value> {
    match value {
        Value::Pointer(pointer) => {
            validate_pointer_base(context, pointer, "ptr_addr")?;
            Ok(Value::I64(pointer.address as i64))
        }
        Value::Heap(reference) => {
            context.heap().get(value)?;
            Ok(Value::I64(reference.address as i64))
        }
        _ => Err(TinyOneError::runtime(
            "ptr_addr() expects a heap value or raw pointer",
        )),
    }
}

pub(crate) fn runtime_pointer_at(context: &TinyRuntimeContext, address: &Value) -> Result<Value> {
    let raw_address = expect_int(address, "ptr_at")?;
    let address = checked_non_negative_usize(raw_address, "heap pointer")?;
    let generation = context.heap().current_generation(address)?;
    Ok(Value::Pointer(RawPointer::new(
        address,
        PointerKind::Object,
        0,
        "",
        generation,
        CastKind::None,
    )))
}

pub(crate) fn runtime_pointer_add(
    context: &TinyRuntimeContext,
    pointer: &Value,
    offset: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_add")?;
    validate_pointer_base(context, &pointer, "ptr_add")?;
    if pointer.kind == PointerKind::Null && pointer.address == 0 {
        return Err(TinyOneError::runtime(
            "Cannot apply pointer arithmetic to null",
        ));
    }
    let offset = expect_int(offset, "ptr_add")?;
    match pointer.kind {
        PointerKind::Object => {
            if offset != 0 {
                return Err(TinyOneError::runtime(
                    "Object pointer arithmetic requires an array or buffer pointer",
                ));
            }
            Ok(Value::Pointer(pointer))
        }
        PointerKind::Array | PointerKind::Buffer => {
            let index = pointer
                .index
                .checked_add(offset)
                .ok_or_else(|| TinyOneError::runtime("ptr_add offset overflow"))?;
            Ok(Value::Pointer(RawPointer::new(
                pointer.address,
                pointer.kind,
                index,
                pointer.field,
                pointer.generation,
                pointer.cast,
            )))
        }
        PointerKind::Field => Err(TinyOneError::runtime(
            "Cannot apply pointer arithmetic to field pointers",
        )),
        PointerKind::Null => Err(TinyOneError::runtime(format!(
            "Unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

pub(crate) fn runtime_pointer_load(context: &TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_load")?;
    if pointer.kind == PointerKind::Null && pointer.address == 0 {
        return Err(TinyOneError::runtime("Cannot load through null"));
    }
    match pointer.kind {
        PointerKind::Object => {
            let heap = context.heap();
            let object = heap.get_address(pointer.address, pointer.generation)?;
            if let HeapData::Cell(bytes) = &object.data {
                Ok(crate::runtime::value_codec::decode_value(
                    bytes.as_slice().try_into().unwrap(),
                ))
            } else {
                drop(heap);
                Ok(Value::Heap(context.heap().ref_at(pointer.address)?))
            }
        }
        PointerKind::Array => {
            let heap = context.heap();
            let object = heap.get_address(pointer.address, pointer.generation)?;
            let HeapData::Array(values) = &object.data else {
                return Err(TinyOneError::runtime(
                    "Array pointer no longer points at an array",
                ));
            };
            let index = checked_collection_index(pointer.index, values.len(), "Array pointer")?;
            values
                .get(index)
                .map(|bytes| crate::runtime::value_codec::decode_value(bytes.try_into().unwrap()))
                .ok_or_else(|| TinyOneError::runtime("Array pointer out of bounds"))
        }
        PointerKind::Buffer => Err(TinyOneError::runtime(
            "Use read8/read16/read32 for buffer pointers",
        )),
        PointerKind::Field => {
            let heap = context.heap();
            let object = heap.get_address(pointer.address, pointer.generation)?;
            let HeapData::Struct(record) = &object.data else {
                return Err(TinyOneError::runtime(
                    "Field pointer no longer points at a struct",
                ));
            };
            record.get(&pointer.field).ok_or_else(|| {
                TinyOneError::runtime(format!(
                    "Unknown field {:?} on struct {:?}",
                    pointer.field, object.type_name
                ))
            })
        }
        PointerKind::Null => Err(TinyOneError::runtime(format!(
            "Unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

pub(crate) fn runtime_pointer_store(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
    value: Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_store")?;
    if pointer.kind == PointerKind::Null && pointer.address == 0 {
        return Err(TinyOneError::runtime("Cannot store through null"));
    }
    match pointer.kind {
        PointerKind::Object => {
            let mut heap = context.heap();
            let object = heap.get_address_mut(pointer.address, pointer.generation)?;
            let HeapData::Cell(cell) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Object raw pointers can only store through pointer cells; use array or field pointers for aggregates",
                ));
            };
            let encoded = crate::runtime::value_codec::encode_value(&value)?;
            cell.as_mut_slice().copy_from_slice(&encoded);
            Ok(value)
        }
        PointerKind::Array => {
            let mut heap = context.heap();
            let object = heap.get_address_mut(pointer.address, pointer.generation)?;
            let HeapData::Array(values) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Array pointer no longer points at an array",
                ));
            };
            let index = checked_collection_index(pointer.index, values.len(), "Array pointer")?;
            let encoded = crate::runtime::value_codec::encode_value(&value)?;
            let target = values
                .get_mut(index)
                .ok_or_else(|| TinyOneError::runtime("Array pointer out of bounds"))?;
            target.copy_from_slice(&encoded);
            Ok(value)
        }
        PointerKind::Buffer => Err(TinyOneError::runtime(
            "Use write8/write16/write32 for buffer pointers",
        )),
        PointerKind::Field => {
            let mut heap = context.heap();
            let object = heap.get_address_mut(pointer.address, pointer.generation)?;
            let type_name = object.type_name.clone();
            let HeapData::Struct(record) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Field pointer no longer points at a struct",
                ));
            };
            if record.set(&pointer.field, &value)? {
                Ok(value)
            } else {
                Err(TinyOneError::runtime(format!(
                    "Unknown field {:?} on struct {type_name:?}",
                    pointer.field
                )))
            }
        }
        _ => Err(TinyOneError::runtime(format!(
            "Unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

pub(crate) fn runtime_pointer_type(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_type")?;
    validate_pointer_base(context, &pointer, "ptr_type")?;
    let text = if pointer.cast == CastKind::None {
        pointer.kind.as_str()
    } else {
        pointer.cast.as_str()
    };
    Ok(Value::Heap(context.heap().alloc_string(text)?))
}

pub(crate) fn runtime_pointer_base(context: &TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_base")?;
    validate_pointer_base(context, &pointer, "ptr_base")?;
    Ok(Value::I64(pointer.address as i64))
}

pub(crate) fn runtime_pointer_offset(
    context: &TinyRuntimeContext,
    pointer: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_offset")?;
    validate_pointer_base(context, &pointer, "ptr_offset")?;
    Ok(Value::I64(
        if pointer.kind == PointerKind::Array || pointer.kind == PointerKind::Buffer {
            pointer.index
        } else {
            0
        },
    ))
}

pub(crate) fn runtime_pointer_kind(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_kind")?;
    validate_pointer_base(context, &pointer, "ptr_kind")?;
    Ok(Value::Heap(
        context.heap().alloc_string(pointer.kind.as_str())?,
    ))
}

pub(crate) fn runtime_pointer_field(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_field")?;
    validate_pointer_base(context, &pointer, "ptr_field")?;
    let field = if pointer.kind == PointerKind::Field {
        pointer.field
    } else {
        String::new()
    };
    Ok(Value::Heap(context.heap().alloc_string(field)?))
}

pub(crate) fn runtime_pointer_eq(
    context: &TinyRuntimeContext,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value> {
    let lhs = expect_pointer(lhs, "ptr_eq")?;
    let rhs = expect_pointer(rhs, "ptr_eq")?;
    validate_pointer_base(context, &lhs, "ptr_eq")?;
    validate_pointer_base(context, &rhs, "ptr_eq")?;
    Ok(Value::I64(
        (pointer_identity(&lhs) == pointer_identity(&rhs)) as i64,
    ))
}

pub(crate) fn runtime_cast_pointer(
    context: &TinyRuntimeContext,
    pointer: &Value,
    type_value: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "cast_ptr")?;
    validate_pointer_base(context, &pointer, "cast_ptr")?;
    let type_name = expect_string(context, type_value, "cast_ptr")?;
    match CastKind::parse(&type_name) {
        Some(cast) => Ok(Value::Pointer(RawPointer::new(
            pointer.address,
            pointer.kind,
            pointer.index,
            pointer.field,
            pointer.generation,
            cast,
        ))),
        None => Err(TinyOneError::runtime(format!(
            "Unsupported pointer cast {type_name:?}"
        ))),
    }
}

pub(crate) fn runtime_make_buffer(context: &mut TinyRuntimeContext, size: &Value) -> Result<Value> {
    let size = checked_bounded_len(
        expect_int(size, "buffer")?,
        "buffer() size",
        MAX_BUFFER_BYTES,
    )?;
    context.heap().ensure_can_allocate(size)?;
    Ok(Value::Heap(context.heap().alloc_buffer(size)?))
}

pub(crate) fn runtime_read_uint(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
    width: usize,
    operation: &str,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, operation)?;
    if pointer.kind == PointerKind::Null && pointer.address == 0 {
        return Err(TinyOneError::runtime(format!(
            "{operation} cannot use null"
        )));
    }
    if pointer.kind != PointerKind::Buffer {
        return Err(TinyOneError::runtime(format!(
            "{operation} expects a buffer pointer"
        )));
    }
    let heap = context.heap();
    let object = heap.get_address(pointer.address, pointer.generation)?;
    let HeapData::Buffer(data) = &object.data else {
        return Err(TinyOneError::runtime(
            "Buffer pointer no longer points at a buffer",
        ));
    };
    let offset = checked_byte_range(pointer.index, width, data.len(), operation)?;
    let bytes = data.as_slice().get(offset..offset + width).ok_or_else(|| {
        TinyOneError::runtime(format!("{operation} out of bounds at byte offset {offset}"))
    })?;
    let mut value = 0u32;
    for (i, byte) in bytes.iter().enumerate() {
        value |= (*byte as u32) << (i * 8);
    }
    Ok(match width {
        1 => Value::U8(value as u8),
        2 => Value::U16(value as u16),
        4 => Value::U32(value),
        _ => {
            return Err(TinyOneError::runtime(format!(
                "{operation} unsupported integer width {width}"
            )));
        }
    })
}

pub(crate) fn runtime_write_uint(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
    value: &Value,
    width: usize,
    operation: &str,
) -> Result<Value> {
    let value_int = expect_int(value, operation)?;
    let max_value = (1i64 << (width * 8)) - 1;
    if value_int < 0 || value_int > max_value {
        return Err(TinyOneError::runtime(format!(
            "{operation} value must be in range 0..{max_value}"
        )));
    }
    let pointer = expect_pointer(pointer, operation)?;
    if pointer.kind == PointerKind::Null && pointer.address == 0 {
        return Err(TinyOneError::runtime(format!(
            "{operation} cannot use null"
        )));
    }
    if pointer.kind != PointerKind::Buffer {
        return Err(TinyOneError::runtime(format!(
            "{operation} expects a buffer pointer"
        )));
    }
    let mut heap = context.heap();
    let object = heap.get_address_mut(pointer.address, pointer.generation)?;
    let HeapData::Buffer(data) = &mut object.data else {
        return Err(TinyOneError::runtime(
            "Buffer pointer no longer points at a buffer",
        ));
    };
    let offset = checked_byte_range(pointer.index, width, data.len(), operation)?;
    let bytes = data
        .as_mut_slice()
        .get_mut(offset..offset + width)
        .ok_or_else(|| {
            TinyOneError::runtime(format!("{operation} out of bounds at byte offset {offset}"))
        })?;
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = ((value_int >> (i * 8)) & 0xff) as u8;
    }
    Ok(match width {
        1 => Value::U8(value_int as u8),
        2 => Value::U16(value_int as u16),
        4 => Value::U32(value_int as u32),
        _ => {
            return Err(TinyOneError::runtime(format!(
                "{operation} unsupported integer width {width}"
            )));
        }
    })
}
