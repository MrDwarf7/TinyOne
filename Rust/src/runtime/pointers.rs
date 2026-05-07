use crate::{
    HeapData, MAX_BUFFER_BYTES, RawPointer, Result, TinyOneError, TinyRuntimeContext, Value,
    checked_bounded_len, checked_byte_range, checked_collection_index, checked_non_negative_usize,
    expect_int, expect_string,
};

pub(crate) fn expect_pointer(value: &Value, operation: &str) -> Result<RawPointer> {
    match value {
        Value::Pointer(pointer) => Ok(pointer.clone()),
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
    if pointer.kind == "null" && pointer.address == 0 {
        return Ok(());
    }
    match pointer.kind.as_str() {
        "object" | "array" | "buffer" | "field" => {
            context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            Ok(())
        }
        _ => Err(TinyOneError::runtime(format!(
            "{operation} got unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

fn pointer_identity(pointer: &RawPointer) -> (usize, u64, String, i64, String) {
    if pointer.kind == "null" && pointer.address == 0 {
        return (0, 0, "null".to_string(), 0, String::new());
    }
    (
        pointer.address,
        pointer.generation,
        pointer.kind.clone(),
        pointer.index,
        pointer.field.clone(),
    )
}

pub(crate) fn runtime_make_pointer(context: &TinyRuntimeContext, args: &[Value]) -> Result<Value> {
    if args.len() == 1 {
        match &args[0] {
            Value::Pointer(pointer) => return Ok(Value::Pointer(pointer.clone())),
            Value::Heap(reference) => {
                context.heap.get(&args[0])?;
                return Ok(Value::Pointer(RawPointer::new(
                    reference.address,
                    "object",
                    0,
                    "",
                    reference.generation,
                    "",
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
    let object = context.heap.get(target)?;
    match object.kind() {
        "array" | "buffer" => Ok(Value::Pointer(RawPointer::new(
            reference.address,
            object.kind(),
            index,
            "",
            reference.generation,
            "",
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
    let object = context.heap.get(target)?;
    let HeapData::Struct(fields) = &object.data else {
        return Err(TinyOneError::runtime(
            "fieldptr() expects a struct heap value",
        ));
    };
    let field = expect_string(context, field_value, "fieldptr")?;
    if !fields.iter().any(|(name, _)| name == &field) {
        return Err(TinyOneError::runtime(format!(
            "Unknown field {field:?} on struct {:?}",
            object.type_name
        )));
    }
    Ok(Value::Pointer(RawPointer::new(
        reference.address,
        "field",
        0,
        field,
        reference.generation,
        "",
    )))
}

pub(crate) fn runtime_pointer_address(
    context: &TinyRuntimeContext,
    value: &Value,
) -> Result<Value> {
    match value {
        Value::Pointer(pointer) => {
            validate_pointer_base(context, pointer, "ptr_addr")?;
            Ok(Value::Int(pointer.address as i64))
        }
        Value::Heap(reference) => {
            context.heap.get(value)?;
            Ok(Value::Int(reference.address as i64))
        }
        _ => Err(TinyOneError::runtime(
            "ptr_addr() expects a heap value or raw pointer",
        )),
    }
}

pub(crate) fn runtime_pointer_at(context: &TinyRuntimeContext, address: &Value) -> Result<Value> {
    let raw_address = expect_int(address, "ptr_at")?;
    let address = checked_non_negative_usize(raw_address, "heap pointer")?;
    let generation = context.heap.current_generation(address)?;
    Ok(Value::Pointer(RawPointer::new(
        address, "object", 0, "", generation, "",
    )))
}

pub(crate) fn runtime_pointer_add(
    context: &TinyRuntimeContext,
    pointer: &Value,
    offset: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_add")?;
    validate_pointer_base(context, &pointer, "ptr_add")?;
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime(
            "Cannot apply pointer arithmetic to null",
        ));
    }
    let offset = expect_int(offset, "ptr_add")?;
    match pointer.kind.as_str() {
        "object" => {
            if offset != 0 {
                return Err(TinyOneError::runtime(
                    "Object pointer arithmetic requires an array or buffer pointer",
                ));
            }
            Ok(Value::Pointer(pointer))
        }
        "array" | "buffer" => {
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
        "field" => Err(TinyOneError::runtime(
            "Cannot apply pointer arithmetic to field pointers",
        )),
        _ => Err(TinyOneError::runtime(format!(
            "Unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

pub(crate) fn runtime_pointer_load(context: &TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_load")?;
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime("Cannot load through null"));
    }
    match pointer.kind.as_str() {
        "object" => {
            let object = context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            if let HeapData::Cell(value) = &object.data {
                Ok(value.clone())
            } else {
                Ok(Value::Heap(context.heap.ref_at(pointer.address)?))
            }
        }
        "array" => {
            let object = context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            let HeapData::Array(values) = &object.data else {
                return Err(TinyOneError::runtime(
                    "Array pointer no longer points at an array",
                ));
            };
            let index = checked_collection_index(pointer.index, values.len(), "Array pointer")?;
            Ok(values[index].clone())
        }
        "buffer" => Err(TinyOneError::runtime(
            "Use read8/read16/read32 for buffer pointers",
        )),
        "field" => {
            let object = context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            let HeapData::Struct(fields) = &object.data else {
                return Err(TinyOneError::runtime(
                    "Field pointer no longer points at a struct",
                ));
            };
            fields
                .iter()
                .find(|(name, _)| name == &pointer.field)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| {
                    TinyOneError::runtime(format!(
                        "Unknown field {:?} on struct {:?}",
                        pointer.field, object.type_name
                    ))
                })
        }
        _ => Err(TinyOneError::runtime(format!(
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
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime("Cannot store through null"));
    }
    match pointer.kind.as_str() {
        "object" => {
            let object = context
                .heap
                .get_address_mut(pointer.address, pointer.generation)?;
            let HeapData::Cell(cell) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Object raw pointers can only store through pointer cells; use array or field pointers for aggregates",
                ));
            };
            *cell = value.clone();
            Ok(value)
        }
        "array" => {
            let object = context
                .heap
                .get_address_mut(pointer.address, pointer.generation)?;
            let HeapData::Array(values) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Array pointer no longer points at an array",
                ));
            };
            let index = checked_collection_index(pointer.index, values.len(), "Array pointer")?;
            values[index] = value.clone();
            Ok(value)
        }
        "buffer" => Err(TinyOneError::runtime(
            "Use write8/write16/write32 for buffer pointers",
        )),
        "field" => {
            let object = context
                .heap
                .get_address_mut(pointer.address, pointer.generation)?;
            let type_name = object.type_name.clone();
            let HeapData::Struct(fields) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Field pointer no longer points at a struct",
                ));
            };
            if let Some((_, field_value)) =
                fields.iter_mut().find(|(name, _)| name == &pointer.field)
            {
                *field_value = value.clone();
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
    let text = if pointer.cast.is_empty() {
        pointer.kind
    } else {
        pointer.cast
    };
    Ok(Value::Heap(context.heap.alloc_string(text)?))
}

pub(crate) fn runtime_pointer_base(context: &TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_base")?;
    validate_pointer_base(context, &pointer, "ptr_base")?;
    Ok(Value::Int(pointer.address as i64))
}

pub(crate) fn runtime_pointer_offset(
    context: &TinyRuntimeContext,
    pointer: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_offset")?;
    validate_pointer_base(context, &pointer, "ptr_offset")?;
    Ok(Value::Int(
        if pointer.kind == "array" || pointer.kind == "buffer" {
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
    Ok(Value::Heap(context.heap.alloc_string(pointer.kind)?))
}

pub(crate) fn runtime_pointer_field(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_field")?;
    validate_pointer_base(context, &pointer, "ptr_field")?;
    let field = if pointer.kind == "field" {
        pointer.field
    } else {
        String::new()
    };
    Ok(Value::Heap(context.heap.alloc_string(field)?))
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
    Ok(Value::Int(
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
    match type_name.as_str() {
        "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => Ok(Value::Pointer(RawPointer::new(
            pointer.address,
            pointer.kind,
            pointer.index,
            pointer.field,
            pointer.generation,
            type_name,
        ))),
        _ => Err(TinyOneError::runtime(format!(
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
    context.heap.ensure_can_allocate(size)?;
    Ok(Value::Heap(context.heap.alloc_buffer(size)?))
}

fn buffer_pointer<'a>(
    context: &'a mut TinyRuntimeContext,
    pointer: &Value,
    operation: &str,
) -> Result<(&'a mut Vec<u8>, i64)> {
    let pointer = expect_pointer(pointer, operation)?;
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime(format!(
            "{operation} cannot use null"
        )));
    }
    if pointer.kind != "buffer" {
        return Err(TinyOneError::runtime(format!(
            "{operation} expects a buffer pointer"
        )));
    }
    let object = context
        .heap
        .get_address_mut(pointer.address, pointer.generation)?;
    let HeapData::Buffer(data) = &mut object.data else {
        return Err(TinyOneError::runtime(
            "Buffer pointer no longer points at a buffer",
        ));
    };
    Ok((data, pointer.index))
}

pub(crate) fn runtime_read_uint(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
    width: usize,
    operation: &str,
) -> Result<Value> {
    let (data, offset) = buffer_pointer(context, pointer, operation)?;
    let offset = checked_byte_range(offset, width, data.len(), operation)?;
    let mut value = 0u32;
    for i in 0..width {
        value |= (data[offset + i] as u32) << (i * 8);
    }
    Ok(Value::Int(value as i64))
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
    let (data, offset) = buffer_pointer(context, pointer, operation)?;
    let offset = checked_byte_range(offset, width, data.len(), operation)?;
    for i in 0..width {
        data[offset + i] = ((value_int >> (i * 8)) & 0xff) as u8;
    }
    Ok(Value::Int(value_int))
}
