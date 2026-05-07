use crate::{
    BUILTINS, HeapData, MAX_ARRAY_LENGTH, Result, TinyOneError, TinyRuntimeContext, VALUE_BYTES,
    Value, checked_bounded_len, checked_payload_bytes, expect_int, expect_pointer, expect_string,
    runtime_array_pop, runtime_array_push, runtime_cast_pointer, runtime_make_buffer,
    runtime_make_field_pointer, runtime_make_pointer, runtime_pointer_add, runtime_pointer_address,
    runtime_pointer_at, runtime_pointer_base, runtime_pointer_eq, runtime_pointer_field,
    runtime_pointer_kind, runtime_pointer_load, runtime_pointer_offset, runtime_pointer_store,
    runtime_pointer_type, runtime_read_uint, runtime_write_uint, validate_pointer_base,
};

pub(crate) fn runtime_call_builtin(
    context: &mut TinyRuntimeContext,
    builtin_index: usize,
    args: Vec<Value>,
) -> Result<Value> {
    let builtin = BUILTINS
        .get(builtin_index)
        .ok_or_else(|| TinyOneError::runtime(format!("Invalid builtin index {builtin_index}")))?;
    if args.len() < builtin.min_args || args.len() > builtin.max_args {
        return Err(TinyOneError::runtime(format!(
            "Builtin {:?} expects {}..{} argument(s), got {}",
            builtin.name,
            builtin.min_args,
            builtin.max_args,
            args.len()
        )));
    }
    match builtin.name {
        "len" => {
            let object = context.heap.get(&args[0])?;
            let len = match &object.data {
                HeapData::Array(values) => values.len(),
                HeapData::String(text) => text.chars().count(),
                HeapData::Buffer(data) => data.len(),
                HeapData::Struct(fields) => fields.len(),
                HeapData::Cell(_) => {
                    return Err(TinyOneError::runtime("len() does not support cell"));
                }
            };
            Ok(Value::Int(len as i64))
        }
        "array" => {
            let count = checked_bounded_len(
                expect_int(&args[0], "array")?,
                "array() length",
                MAX_ARRAY_LENGTH,
            )?;
            let bytes = checked_payload_bytes(count, VALUE_BYTES, "array()")?;
            context.heap.ensure_can_allocate(bytes)?;
            Ok(Value::Heap(
                context.heap.alloc_array(vec![args[1].clone(); count])?,
            ))
        }
        "alloc" => Ok(Value::Heap(context.heap.alloc_cell(args[0].clone())?)),
        "load" => {
            let object = context.heap.get(&args[0])?;
            let HeapData::Cell(value) = &object.data else {
                return Err(TinyOneError::runtime("load() expects a pointer cell"));
            };
            Ok(value.clone())
        }
        "store" => {
            let object = context.heap.get_mut(&args[0])?;
            let HeapData::Cell(value) = &mut object.data else {
                return Err(TinyOneError::runtime("store() expects a pointer cell"));
            };
            *value = args[1].clone();
            Ok(args[1].clone())
        }
        "free" => {
            context.heap.free(&args[0])?;
            Ok(Value::Int(0))
        }
        "read" => {
            let raw = context.read_raw()?;
            if looks_like_int(&raw) {
                Ok(Value::Int(raw.parse().map_err(|_| {
                    TinyOneError::runtime("read() integer input is out of range")
                })?))
            } else {
                Ok(Value::Heap(context.heap.alloc_string(raw)?))
            }
        }
        "read_int" => {
            let raw = context.read_raw()?;
            if !looks_like_int(&raw) {
                return Err(TinyOneError::runtime(format!(
                    "read_int() expected integer input, got {raw:?}"
                )));
            }
            Ok(Value::Int(raw.parse().map_err(|_| {
                TinyOneError::runtime("read_int() integer input is out of range")
            })?))
        }
        "read_str" => {
            let raw = context.read_raw()?;
            Ok(Value::Heap(context.heap.alloc_string(raw)?))
        }
        "to_int" => match &args[0] {
            Value::Int(value) => Ok(Value::Int(*value)),
            _ => {
                let text = expect_string(context, &args[0], "to_int")?;
                if !looks_like_int(&text) {
                    return Err(TinyOneError::runtime(
                        "to_int() expects an integer or numeric string",
                    ));
                }
                Ok(Value::Int(text.parse().map_err(|_| {
                    TinyOneError::runtime("to_int() integer input is out of range")
                })?))
            }
        },
        "ptr" => runtime_make_pointer(context, &args),
        "fieldptr" => runtime_make_field_pointer(context, &args[0], &args[1]),
        "ptr_addr" => runtime_pointer_address(context, &args[0]),
        "ptr_at" => runtime_pointer_at(context, &args[0]),
        "ptr_add" => runtime_pointer_add(context, &args[0], &args[1]),
        "ptr_load" => runtime_pointer_load(context, &args[0]),
        "ptr_store" => runtime_pointer_store(context, &args[0], args[1].clone()),
        "ptr_type" => runtime_pointer_type(context, &args[0]),
        "buffer" => runtime_make_buffer(context, &args[0]),
        "is_null" => {
            let pointer = expect_pointer(&args[0], "is_null")?;
            validate_pointer_base(context, &pointer, "is_null")?;
            Ok(Value::Int(
                (pointer.kind == "null" && pointer.address == 0) as i64,
            ))
        }
        "ptr_eq" => runtime_pointer_eq(context, &args[0], &args[1]),
        "ptr_ne" => match runtime_pointer_eq(context, &args[0], &args[1])? {
            Value::Int(0) => Ok(Value::Int(1)),
            _ => Ok(Value::Int(0)),
        },
        "ptr_base" => runtime_pointer_base(context, &args[0]),
        "ptr_offset" => runtime_pointer_offset(context, &args[0]),
        "ptr_kind" => runtime_pointer_kind(context, &args[0]),
        "ptr_field" => runtime_pointer_field(context, &args[0]),
        "read8" => runtime_read_uint(context, &args[0], 1, "read8"),
        "write8" => runtime_write_uint(context, &args[0], &args[1], 1, "write8"),
        "read16" => runtime_read_uint(context, &args[0], 2, "read16"),
        "write16" => runtime_write_uint(context, &args[0], &args[1], 2, "write16"),
        "read32" => runtime_read_uint(context, &args[0], 4, "read32"),
        "write32" => runtime_write_uint(context, &args[0], &args[1], 4, "write32"),
        "cast_ptr" => runtime_cast_pointer(context, &args[0], &args[1]),
        "push" => runtime_array_push(context, &args[0], args[1].clone()),
        "pop" => runtime_array_pop(context, &args[0]),
        _ => Err(TinyOneError::runtime(format!(
            "Missing builtin handler {:?}",
            builtin.name
        ))),
    }
}

fn looks_like_int(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let rest = text.strip_prefix(['+', '-']).unwrap_or(text);
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
}
