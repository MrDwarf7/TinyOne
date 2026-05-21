use crate::runtime::stdlib;
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
            let heap = context.heap();
            let object = heap.get(&args[0])?;
            let len = match &object.data {
                HeapData::Array(values) => values.len(),
                HeapData::String(text) => text.chars().count(),
                HeapData::Buffer(data) => data.len(),
                HeapData::Struct(fields) => fields.len(),
                HeapData::Map(entries) => entries.len(),
                HeapData::Cell(_) => {
                    return Err(TinyOneError::runtime("len() does not support cell"));
                }
                _ => {
                    return Err(TinyOneError::runtime(format!(
                        "len() does not support {}",
                        object.kind()
                    )));
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
            context.heap().ensure_can_allocate(bytes)?;
            Ok(Value::Heap(
                context.heap().alloc_array(vec![args[1].clone(); count])?,
            ))
        }
        "alloc" => Ok(Value::Heap(context.heap().alloc_cell(args[0].clone())?)),
        "load" => {
            let heap = context.heap();
            let object = heap.get(&args[0])?;
            let HeapData::Cell(value) = &object.data else {
                return Err(TinyOneError::runtime("load() expects a pointer cell"));
            };
            Ok(value.clone())
        }
        "store" => {
            let mut heap = context.heap();
            let object = heap.get_mut(&args[0])?;
            let HeapData::Cell(value) = &mut object.data else {
                return Err(TinyOneError::runtime("store() expects a pointer cell"));
            };
            *value = args[1].clone();
            Ok(args[1].clone())
        }
        "free" => {
            context.heap().free(&args[0])?;
            Ok(Value::Int(0))
        }
        "read" => {
            let raw = context.read_raw()?;
            if looks_like_int(&raw) {
                Ok(Value::Int(raw.parse().map_err(|_| {
                    TinyOneError::runtime("read() integer input is out of range")
                })?))
            } else {
                Ok(Value::Heap(context.heap().alloc_string(raw)?))
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
            Ok(Value::Heap(context.heap().alloc_string(raw)?))
        }
        "to_int" => match &args[0] {
            Value::Int(value) => Ok(Value::Int(*value)),
            Value::U8(value) => Ok(Value::Int(*value as i64)),
            Value::U16(value) => Ok(Value::Int(*value as i64)),
            Value::U32(value) => Ok(Value::Int(*value as i64)),
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
        // -------- Phase 2 stdlib --------
        "vec_new" => stdlib::b_vec_new(context),
        "vec_clear" => stdlib::b_vec_clear(context, &args[0]),
        "map_new" => stdlib::b_map_new(context),
        "map_set" => stdlib::b_map_set(context, &args[0], args[1].clone(), args[2].clone()),
        "map_get" => stdlib::b_map_get(context, &args[0], &args[1]),
        "map_has" => stdlib::b_map_has(context, &args[0], &args[1]),
        "map_del" => stdlib::b_map_del(context, &args[0], &args[1]),
        "map_len" => stdlib::b_map_len(context, &args[0]),
        "map_keys" => stdlib::b_map_keys(context, &args[0]),
        "map_values" => stdlib::b_map_values(context, &args[0]),
        "io_stdout" => Ok(stdlib::b_io_stdout()),
        "io_stderr" => Ok(stdlib::b_io_stderr()),
        "io_stdin" => Ok(stdlib::b_io_stdin()),
        "io_write" => stdlib::b_io_write(context, &args[0], &args[1]),
        "io_writeln" => stdlib::b_io_writeln(context, &args[0], &args[1]),
        "io_read_line" => stdlib::b_io_read_line(context),
        "io_flush" => stdlib::b_io_flush(context, &args[0]),
        "io_capture_stdout" => stdlib::b_io_capture_stdout(context),
        "io_capture_stderr" => stdlib::b_io_capture_stderr(context),
        "str_byte_len" => stdlib::b_str_byte_len(context, &args[0]),
        "str_char_len" => stdlib::b_str_char_len(context, &args[0]),
        "str_byte_at" => stdlib::b_str_byte_at(context, &args[0], &args[1]),
        "str_char_at" => stdlib::b_str_char_at(context, &args[0], &args[1]),
        "str_slice" => stdlib::b_str_slice(context, &args[0], &args[1], &args[2]),
        "str_concat" => stdlib::b_str_concat(context, &args[0], &args[1]),
        "str_is_utf8" => stdlib::b_str_is_utf8(context, &args[0]),
        "str_from_buffer" => stdlib::b_str_from_buffer(context, &args[0]),
        "mutex_new" => stdlib::b_mutex_new(context),
        "mutex_lock" => stdlib::b_mutex_lock(context, &args[0]),
        "mutex_unlock" => stdlib::b_mutex_unlock(context, &args[0]),
        "atomic_new" => stdlib::b_atomic_new(context, &args[0]),
        "atomic_load" => stdlib::b_atomic_load(context, &args[0]),
        "atomic_store" => stdlib::b_atomic_store(context, &args[0], &args[1]),
        "atomic_add" => stdlib::b_atomic_add(context, &args[0], &args[1]),
        "result_ok" => stdlib::b_result_ok(context, args[0].clone()),
        "result_err" => stdlib::b_result_err(context, args[0].clone()),
        "result_is_ok" => stdlib::b_result_is_ok(context, &args[0]),
        "result_is_err" => stdlib::b_result_is_err(context, &args[0]),
        "result_unwrap" => stdlib::b_result_unwrap(context, &args[0]),
        "result_unwrap_err" => stdlib::b_result_unwrap_err(context, &args[0]),
        "option_some" => stdlib::b_option_some(context, args[0].clone()),
        "option_none" => stdlib::b_option_none(context),
        "option_is_some" => stdlib::b_option_is_some(context, &args[0]),
        "option_is_none" => stdlib::b_option_is_none(context, &args[0]),
        "option_unwrap" => stdlib::b_option_unwrap(context, &args[0]),
        "sys_argc" => stdlib::b_sys_argc(context),
        "sys_argv" => stdlib::b_sys_argv(context, &args[0]),
        "sys_env_has" => stdlib::b_sys_env_has(context, &args[0]),
        "sys_env_get" => stdlib::b_sys_env_get(context, &args[0]),
        "path_join" => stdlib::b_path_join(context, &args[0], &args[1]),
        "path_basename" => stdlib::b_path_basename(context, &args[0]),
        "path_dirname" => stdlib::b_path_dirname(context, &args[0]),
        "fs_read" => stdlib::b_fs_read(context, &args[0]),
        "fs_write" => stdlib::b_fs_write(context, &args[0], &args[1]),
        "fs_exists" => stdlib::b_fs_exists(context, &args[0]),
        "fs_list_dir" => stdlib::b_fs_list_dir(context, &args[0]),
        "math_const" => stdlib::b_math_const(context, &args[0]),
        "math_abs" => stdlib::b_math_abs(&args[0]),
        "math_min" => stdlib::b_math_min(&args[0], &args[1]),
        "math_max" => stdlib::b_math_max(&args[0], &args[1]),
        "logic_and" => stdlib::b_logic_and(&args[0], &args[1]),
        "logic_or" => stdlib::b_logic_or(&args[0], &args[1]),
        "logic_not" => stdlib::b_logic_not(&args[0]),
        "logic_xor" => stdlib::b_logic_xor(&args[0], &args[1]),
        "type_of" => stdlib::b_type_of(context, &args[0]),
        "type_id" => stdlib::b_type_id(context, &args[0]),
        "smallest_fit" => stdlib::b_smallest_fit(&args[0], context),
        "promote" => stdlib::b_promote(context, &args[0], &args[1]),
        "check_int_range" => stdlib::b_check_int_range(context, &args[0], &args[1]),
        "typed_add" => stdlib::b_typed_add(context, &args[0], &args[1], &args[2]),
        "typed_sub" => stdlib::b_typed_sub(context, &args[0], &args[1], &args[2]),
        "typed_mul" => stdlib::b_typed_mul(context, &args[0], &args[1], &args[2]),
        "typed_div" => stdlib::b_typed_div(context, &args[0], &args[1], &args[2]),
        "typed_neg" => stdlib::b_typed_neg(context, &args[0], &args[1]),
        "i64" => stdlib::b_int_cast(&args[0], crate::TypeKind::I64, "i64"),
        "u8" => stdlib::b_int_cast(&args[0], crate::TypeKind::U8, "u8"),
        "u16" => stdlib::b_int_cast(&args[0], crate::TypeKind::U16, "u16"),
        "u32" => stdlib::b_int_cast(&args[0], crate::TypeKind::U32, "u32"),
        "assert" => stdlib::b_assert(&args[0], args.get(1), context),
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
