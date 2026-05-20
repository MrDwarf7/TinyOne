//! Stdlib bridge layer.
//!
//! All builtins added after the initial 35 are dispatched from
//! [`runtime_call_stdlib_builtin`]. They are bytecode-stable: their
//! definitions live in [`crate::builtins::BUILTINS`] after index 34.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;

use crate::runtime::typing::{
    TypeKind, check_integer_range, integer_range, promote_integer, smallest_fit_literal,
};
use crate::{
    HeapData, Result, TinyOneError, TinyRuntimeContext, VALUE_BYTES, Value, expect_int,
    expect_string, runtime_cast_int, runtime_integer_kind, runtime_integer_value,
    validate_pointer_base,
};

const MAX_FS_LIST_DIR_ENTRIES: usize = 65_536;

fn expect_kind(value: &Value, kind: &str, operation: &str) -> Result<i64> {
    let Value::Int(value) = value else {
        return Err(TinyOneError::runtime(format!("{operation} expects {kind}")));
    };
    Ok(*value)
}

fn parse_type_name(text: &str, operation: &str) -> Result<TypeKind> {
    TypeKind::from_name(text)
        .ok_or_else(|| TinyOneError::runtime(format!("{operation} unknown type name {:?}", text)))
}

fn runtime_integer_type_name(value: &Value) -> Option<&'static str> {
    runtime_integer_kind(value).map(TypeKind::name)
}

pub fn b_int_cast(value: &Value, kind: TypeKind, operation: &str) -> Result<Value> {
    runtime_cast_int(value, kind, operation)
}

// ---------------------------------------------------------------------------
// Vec helpers (vec_new, vec_push, vec_pop, vec_get, vec_set, vec_len)
//
// A TinyOne Vec is a heap-array used through the existing array machinery
// but accessed via length-aware safe builtins. Existing `array`, `push`,
// `pop`, `len`, and indexing builtins continue to work without modification.
// ---------------------------------------------------------------------------

pub fn b_vec_new(context: &mut TinyRuntimeContext) -> Result<Value> {
    Ok(Value::Heap(context.heap.alloc_array(Vec::new())?))
}

pub fn b_vec_clear(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let cleared = {
        let object = context.heap.get_mut(target)?;
        let HeapData::Array(values) = &mut object.data else {
            return Err(TinyOneError::runtime("vec_clear expects a vec/array"));
        };
        let cleared = values.len();
        values.clear();
        cleared
    };
    context
        .heap
        .record_shrink(cleared.saturating_mul(VALUE_BYTES))?;
    Ok(Value::Int(0))
}

// ---------------------------------------------------------------------------
// Map helpers (map_new, map_set, map_get, map_has, map_del, map_len, map_keys)
//
// Maps are stored as a new HeapData variant Map(Vec<(Value, Value)>) so
// iteration order is the insertion order; the spec requires this.
// ---------------------------------------------------------------------------

pub fn b_map_new(context: &mut TinyRuntimeContext) -> Result<Value> {
    Ok(Value::Heap(context.heap.alloc_map(Vec::new())?))
}

pub fn b_map_set(
    context: &mut TinyRuntimeContext,
    target: &Value,
    key: Value,
    value: Value,
) -> Result<Value> {
    // Look up existing index without holding a mutable borrow across compare.
    let mut existing: Option<usize> = None;
    {
        let object = context.heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_set expects a map"));
        };
        for (idx, (k, _)) in entries.iter().enumerate() {
            if map_key_equal(context, k, &key)? {
                existing = Some(idx);
                break;
            }
        }
    }
    if existing.is_none() {
        context
            .heap
            .ensure_can_allocate_delta(VALUE_BYTES.saturating_mul(2))?;
    }
    let mut inserted = false;
    {
        let object = context.heap.get_mut(target)?;
        let HeapData::Map(entries) = &mut object.data else {
            return Err(TinyOneError::runtime("map_set expects a map"));
        };
        if let Some(idx) = existing {
            let Some((_, existing_value)) = entries.get_mut(idx) else {
                return Err(TinyOneError::runtime("map_set: internal index error"));
            };
            *existing_value = value.clone();
        } else {
            entries.push((key, value.clone()));
            inserted = true;
        }
    }
    if inserted {
        context.heap.record_growth(VALUE_BYTES.saturating_mul(2))?;
    }
    Ok(value)
}

pub fn b_map_get(context: &mut TinyRuntimeContext, target: &Value, key: &Value) -> Result<Value> {
    let object = context.heap.get(target)?;
    let HeapData::Map(entries) = &object.data else {
        return Err(TinyOneError::runtime("map_get expects a map"));
    };
    for (k, v) in entries.iter() {
        if map_key_equal(context, k, key)? {
            return Ok(v.clone());
        }
    }
    Err(TinyOneError::runtime("map_get: missing key"))
}

pub fn b_map_has(context: &TinyRuntimeContext, target: &Value, key: &Value) -> Result<Value> {
    let object = context.heap.get(target)?;
    let HeapData::Map(entries) = &object.data else {
        return Err(TinyOneError::runtime("map_has expects a map"));
    };
    for (k, _) in entries {
        if map_key_equal(context, k, key)? {
            return Ok(Value::Int(1));
        }
    }
    Ok(Value::Int(0))
}

pub fn b_map_del(context: &mut TinyRuntimeContext, target: &Value, key: &Value) -> Result<Value> {
    let to_remove: Option<usize> = {
        let object = context.heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_del expects a map"));
        };
        let mut found = None;
        for (idx, (k, _)) in entries.iter().enumerate() {
            if map_key_equal(context, k, key)? {
                found = Some(idx);
                break;
            }
        }
        found
    };
    let removed = if let Some(idx) = to_remove {
        let object = context.heap.get_mut(target)?;
        let HeapData::Map(entries) = &mut object.data else {
            return Err(TinyOneError::runtime("map_del expects a map"));
        };
        entries.remove(idx);
        true
    } else {
        false
    };
    if removed {
        context.heap.record_shrink(VALUE_BYTES.saturating_mul(2))?;
        Ok(Value::Int(1))
    } else {
        Ok(Value::Int(0))
    }
}

pub fn b_map_len(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let object = context.heap.get(target)?;
    let HeapData::Map(entries) = &object.data else {
        return Err(TinyOneError::runtime("map_len expects a map"));
    };
    Ok(Value::Int(entries.len() as i64))
}

pub fn b_map_keys(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let keys: Vec<Value> = {
        let object = context.heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_keys expects a map"));
        };
        entries.iter().map(|(k, _)| k.clone()).collect()
    };
    Ok(Value::Heap(context.heap.alloc_array(keys)?))
}

pub fn b_map_values(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let values: Vec<Value> = {
        let object = context.heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_values expects a map"));
        };
        entries.iter().map(|(_, v)| v.clone()).collect()
    };
    Ok(Value::Heap(context.heap.alloc_array(values)?))
}

fn map_key_equal(context: &TinyRuntimeContext, lhs: &Value, rhs: &Value) -> Result<bool> {
    match (lhs, rhs) {
        (
            Value::Int(_) | Value::U8(_) | Value::U16(_) | Value::U32(_),
            Value::Int(_) | Value::U8(_) | Value::U16(_) | Value::U32(_),
        ) => Ok(runtime_integer_value(lhs, "map key")? == runtime_integer_value(rhs, "map key")?),
        (Value::Pointer(a), Value::Pointer(b)) => {
            validate_pointer_base(context, a, "map key")?;
            validate_pointer_base(context, b, "map key")?;
            Ok(a.kind == b.kind
                && a.address == b.address
                && a.generation == b.generation
                && a.index == b.index
                && a.field == b.field)
        }
        (Value::Heap(_), Value::Heap(_)) => {
            // Strings are interned by content for map equality; this matches
            // typing_system.md's "keys must support stable equality" rule.
            let lhs_obj = context.heap.get(lhs);
            let rhs_obj = context.heap.get(rhs);
            match (lhs_obj, rhs_obj) {
                (Ok(a), Ok(b)) => match (&a.data, &b.data) {
                    (HeapData::String(left), HeapData::String(right)) => Ok(left == right),
                    _ => match (lhs, rhs) {
                        (Value::Heap(la), Value::Heap(rb)) => {
                            Ok(la.address == rb.address && la.generation == rb.generation)
                        }
                        _ => Ok(false),
                    },
                },
                _ => Ok(false),
            }
        }
        _ => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// I/O bridge: stdin/stdout/stderr behave deterministically.
//
// The runtime context carries an injected stdout/stderr buffer. `io_write`
// appends to either; `io_stdin_line` consumes one element from the
// deterministic input queue (the same one used by `read`).
// ---------------------------------------------------------------------------

pub const IO_FD_STDOUT: i64 = 1;
pub const IO_FD_STDERR: i64 = 2;
pub const IO_FD_STDIN: i64 = 0;

pub fn b_io_stdout() -> Value {
    Value::Int(IO_FD_STDOUT)
}
pub fn b_io_stderr() -> Value {
    Value::Int(IO_FD_STDERR)
}
pub fn b_io_stdin() -> Value {
    Value::Int(IO_FD_STDIN)
}

pub fn b_io_write(
    context: &mut TinyRuntimeContext,
    fd: &Value,
    text_value: &Value,
) -> Result<Value> {
    let fd = expect_kind(fd, "an integer file descriptor", "io_write")?;
    let text = expect_string(context, text_value, "io_write")?;
    let bytes = text.len() as i64;
    match fd {
        IO_FD_STDOUT => context.io_stdout.push_str(&text),
        IO_FD_STDERR => context.io_stderr.push_str(&text),
        IO_FD_STDIN => {
            return Err(TinyOneError::runtime(
                "io_write: cannot write to stdin (fd 0)",
            ));
        }
        _ => {
            return Err(TinyOneError::runtime(format!(
                "io_write: unsupported fd {fd}"
            )));
        }
    }
    Ok(Value::Int(bytes))
}

pub fn b_io_writeln(
    context: &mut TinyRuntimeContext,
    fd: &Value,
    text_value: &Value,
) -> Result<Value> {
    let fd = expect_kind(fd, "an integer file descriptor", "io_writeln")?;
    let text = expect_string(context, text_value, "io_writeln")?;
    let bytes = text.len() as i64 + 1;
    match fd {
        IO_FD_STDOUT => {
            context.io_stdout.push_str(&text);
            context.io_stdout.push('\n');
        }
        IO_FD_STDERR => {
            context.io_stderr.push_str(&text);
            context.io_stderr.push('\n');
        }
        IO_FD_STDIN => {
            return Err(TinyOneError::runtime(
                "io_writeln: cannot write to stdin (fd 0)",
            ));
        }
        _ => {
            return Err(TinyOneError::runtime(format!(
                "io_writeln: unsupported fd {fd}"
            )));
        }
    }
    Ok(Value::Int(bytes))
}

pub fn b_io_read_line(context: &mut TinyRuntimeContext) -> Result<Value> {
    let raw = context.read_raw()?;
    Ok(Value::Heap(context.heap.alloc_string(raw)?))
}

pub fn b_io_flush(_context: &mut TinyRuntimeContext, _fd: &Value) -> Result<Value> {
    // No-op for deterministic test doubles. Flushing the real stdout still
    // happens through the host once `VM::run` returns.
    Ok(Value::Int(0))
}

pub fn b_io_capture_stdout(context: &mut TinyRuntimeContext) -> Result<Value> {
    let text = std::mem::take(&mut context.io_stdout);
    Ok(Value::Heap(context.heap.alloc_string(text)?))
}

pub fn b_io_capture_stderr(context: &mut TinyRuntimeContext) -> Result<Value> {
    let text = std::mem::take(&mut context.io_stderr);
    Ok(Value::Heap(context.heap.alloc_string(text)?))
}

// ---------------------------------------------------------------------------
// String & Unicode helpers.
// ---------------------------------------------------------------------------

pub fn b_str_byte_len(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let text = expect_string(context, target, "str_byte_len")?;
    Ok(Value::Int(text.len() as i64))
}

pub fn b_str_char_len(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let text = expect_string(context, target, "str_char_len")?;
    Ok(Value::Int(text.chars().count() as i64))
}

pub fn b_str_byte_at(context: &TinyRuntimeContext, target: &Value, index: &Value) -> Result<Value> {
    let text = expect_string(context, target, "str_byte_at")?;
    let index = expect_int(index, "str_byte_at")?;
    if index < 0 {
        return Err(TinyOneError::runtime("str_byte_at: negative index"));
    }
    let index = usize::try_from(index)
        .map_err(|_| TinyOneError::runtime("str_byte_at: index is too large"))?;
    let bytes = text.as_bytes();
    if index >= bytes.len() {
        return Err(TinyOneError::runtime("str_byte_at: index out of bounds"));
    }
    let byte = bytes
        .get(index)
        .ok_or_else(|| TinyOneError::runtime("str_byte_at: index out of bounds"))?;
    Ok(Value::Int(*byte as i64))
}

pub fn b_str_char_at(
    context: &mut TinyRuntimeContext,
    target: &Value,
    index: &Value,
) -> Result<Value> {
    let text = expect_string(context, target, "str_char_at")?;
    let index = expect_int(index, "str_char_at")?;
    if index < 0 {
        return Err(TinyOneError::runtime("str_char_at: negative index"));
    }
    let index = usize::try_from(index)
        .map_err(|_| TinyOneError::runtime("str_char_at: index is too large"))?;
    let ch = text
        .chars()
        .nth(index)
        .ok_or_else(|| TinyOneError::runtime("str_char_at: index out of bounds"))?;
    Ok(Value::Heap(context.heap.alloc_string(ch.to_string())?))
}

pub fn b_str_slice(
    context: &mut TinyRuntimeContext,
    target: &Value,
    start: &Value,
    end: &Value,
) -> Result<Value> {
    let text = expect_string(context, target, "str_slice")?;
    let start = expect_int(start, "str_slice")?;
    let end = expect_int(end, "str_slice")?;
    if start < 0 || end < 0 {
        return Err(TinyOneError::runtime("str_slice: negative bound"));
    }
    if end < start {
        return Err(TinyOneError::runtime("str_slice: end < start"));
    }
    let text_bytes = text.len();
    let total_chars = i64::try_from(text.chars().count())
        .map_err(|_| TinyOneError::runtime("str_slice: string is too large"))?;
    if start > total_chars || end > total_chars {
        return Err(TinyOneError::runtime("str_slice: bound out of range"));
    }
    let char_byte_offset = |target: i64| -> Result<usize> {
        if target == total_chars {
            Ok(text_bytes)
        } else {
            let target = usize::try_from(target)
                .map_err(|_| TinyOneError::runtime("str_slice: bound is too large"))?;
            text.char_indices()
                .nth(target)
                .map(|(byte_index, _)| byte_index)
                .ok_or_else(|| TinyOneError::runtime("str_slice: bound out of range"))
        }
    };
    let byte_start = char_byte_offset(start)?;
    let byte_end = char_byte_offset(end)?;
    let sliced = text
        .get(byte_start..byte_end)
        .ok_or_else(|| TinyOneError::runtime("str_slice: byte boundary not on char boundary"))?
        .to_string();
    Ok(Value::Heap(context.heap.alloc_string(sliced)?))
}

pub fn b_str_concat(
    context: &mut TinyRuntimeContext,
    left: &Value,
    right: &Value,
) -> Result<Value> {
    let mut left = expect_string(context, left, "str_concat")?;
    let right = expect_string(context, right, "str_concat")?;
    left.push_str(&right);
    Ok(Value::Heap(context.heap.alloc_string(left)?))
}

pub fn b_str_is_utf8(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    // String values in TinyOne are always UTF-8 by construction. Probe a
    // buffer instead so external bytes can be validated before becoming a
    // String. If the target is a buffer we run std::str::from_utf8 on its
    // bytes.
    if let Ok(text) = expect_string(context, target, "str_is_utf8") {
        let _ = text;
        return Ok(Value::Int(1));
    }
    let object = context.heap.get(target)?;
    let HeapData::Buffer(bytes) = &object.data else {
        return Err(TinyOneError::runtime(
            "str_is_utf8 expects a String or Buffer",
        ));
    };
    Ok(Value::Int(std::str::from_utf8(bytes).is_ok() as i64))
}

pub fn b_str_from_buffer(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let bytes = {
        let object = context.heap.get(target)?;
        let HeapData::Buffer(bytes) = &object.data else {
            return Err(TinyOneError::runtime("str_from_buffer expects a Buffer"));
        };
        bytes.clone()
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| TinyOneError::runtime("str_from_buffer: invalid UTF-8"))?
        .to_string();
    Ok(Value::Heap(context.heap.alloc_string(text)?))
}

// ---------------------------------------------------------------------------
// Threading & sync (single-thread semantic shells).
//
// `Mutex` and `Atomic` are heap-allocated cells with a small protocol. They
// keep the VM honest about misuse (double-lock, unlock-when-unlocked) so
// programs validate the same way on both runtimes.
// ---------------------------------------------------------------------------

pub fn b_mutex_new(context: &mut TinyRuntimeContext) -> Result<Value> {
    let inner = context.heap.alloc_struct(
        "tinyone.sync.Mutex",
        vec![("locked".to_string(), Value::Int(0))],
    )?;
    Ok(Value::Heap(inner))
}

pub fn b_mutex_lock(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let object = context.heap.get_mut(target)?;
    let HeapData::Struct(fields) = &mut object.data else {
        return Err(TinyOneError::runtime("mutex_lock expects a Mutex"));
    };
    let entry = fields
        .iter_mut()
        .find(|(name, _)| name == "locked")
        .ok_or_else(|| TinyOneError::runtime("mutex_lock: missing locked slot"))?;
    let Value::Int(state) = &mut entry.1 else {
        return Err(TinyOneError::runtime("mutex_lock: corrupt mutex state"));
    };
    if *state != 0 {
        return Err(TinyOneError::runtime(
            "mutex_lock: already locked (deadlock)",
        ));
    }
    *state = 1;
    Ok(Value::Int(1))
}

pub fn b_mutex_unlock(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let object = context.heap.get_mut(target)?;
    let HeapData::Struct(fields) = &mut object.data else {
        return Err(TinyOneError::runtime("mutex_unlock expects a Mutex"));
    };
    let entry = fields
        .iter_mut()
        .find(|(name, _)| name == "locked")
        .ok_or_else(|| TinyOneError::runtime("mutex_unlock: missing locked slot"))?;
    let Value::Int(state) = &mut entry.1 else {
        return Err(TinyOneError::runtime("mutex_unlock: corrupt mutex state"));
    };
    if *state == 0 {
        return Err(TinyOneError::runtime("mutex_unlock: not locked"));
    }
    *state = 0;
    Ok(Value::Int(0))
}

pub fn b_atomic_new(context: &mut TinyRuntimeContext, init: &Value) -> Result<Value> {
    let init = expect_int(init, "atomic_new")?;
    let inner = context.heap.alloc_struct(
        "tinyone.sync.Atomic",
        vec![("value".to_string(), Value::Int(init))],
    )?;
    Ok(Value::Heap(inner))
}

pub fn b_atomic_load(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let object = context.heap.get(target)?;
    let HeapData::Struct(fields) = &object.data else {
        return Err(TinyOneError::runtime("atomic_load expects an Atomic"));
    };
    let entry = fields
        .iter()
        .find(|(name, _)| name == "value")
        .ok_or_else(|| TinyOneError::runtime("atomic_load: missing value slot"))?;
    Ok(entry.1.clone())
}

pub fn b_atomic_store(
    context: &mut TinyRuntimeContext,
    target: &Value,
    new_value: &Value,
) -> Result<Value> {
    let new_value = expect_int(new_value, "atomic_store")?;
    let object = context.heap.get_mut(target)?;
    let HeapData::Struct(fields) = &mut object.data else {
        return Err(TinyOneError::runtime("atomic_store expects an Atomic"));
    };
    let entry = fields
        .iter_mut()
        .find(|(name, _)| name == "value")
        .ok_or_else(|| TinyOneError::runtime("atomic_store: missing value slot"))?;
    entry.1 = Value::Int(new_value);
    Ok(Value::Int(new_value))
}

pub fn b_atomic_add(
    context: &mut TinyRuntimeContext,
    target: &Value,
    delta: &Value,
) -> Result<Value> {
    let delta = expect_int(delta, "atomic_add")?;
    let object = context.heap.get_mut(target)?;
    let HeapData::Struct(fields) = &mut object.data else {
        return Err(TinyOneError::runtime("atomic_add expects an Atomic"));
    };
    let entry = fields
        .iter_mut()
        .find(|(name, _)| name == "value")
        .ok_or_else(|| TinyOneError::runtime("atomic_add: missing value slot"))?;
    let Value::Int(current) = entry.1 else {
        return Err(TinyOneError::runtime("atomic_add: corrupt atomic state"));
    };
    let next = current
        .checked_add(delta)
        .ok_or_else(|| TinyOneError::runtime("Runtime.Memory_Overflow: atomic_add overflow"))?;
    entry.1 = Value::Int(next);
    Ok(Value::Int(next))
}

// ---------------------------------------------------------------------------
// Result / Option.
//
// Variants are heap structs because TinyOne does not yet have surface sum-type
// syntax. Tag values: 0 = Err/None, 1 = Ok/Some. This is documented and
// version-controlled in typing_system.md alignment.
// ---------------------------------------------------------------------------

pub const VARIANT_OK: i64 = 1;
pub const VARIANT_ERR: i64 = 0;
pub const VARIANT_SOME: i64 = 1;
pub const VARIANT_NONE: i64 = 0;

pub fn b_result_ok(context: &mut TinyRuntimeContext, payload: Value) -> Result<Value> {
    Ok(Value::Heap(context.heap.alloc_struct(
        "tinyone.result.Result",
        vec![
            ("tag".to_string(), Value::Int(VARIANT_OK)),
            ("payload".to_string(), payload),
        ],
    )?))
}

pub fn b_result_err(context: &mut TinyRuntimeContext, payload: Value) -> Result<Value> {
    Ok(Value::Heap(context.heap.alloc_struct(
        "tinyone.result.Result",
        vec![
            ("tag".to_string(), Value::Int(VARIANT_ERR)),
            ("payload".to_string(), payload),
        ],
    )?))
}

fn variant_field<'a>(
    context: &'a TinyRuntimeContext,
    target: &Value,
    type_name: &str,
    field: &str,
    operation: &str,
) -> Result<&'a Value> {
    let object = context.heap.get(target)?;
    if object.type_name != type_name {
        return Err(TinyOneError::runtime(format!(
            "{operation}: expected {type_name}, got {:?}",
            object.type_name
        )));
    }
    let HeapData::Struct(fields) = &object.data else {
        return Err(TinyOneError::runtime(format!(
            "{operation}: corrupt {type_name}"
        )));
    };
    fields
        .iter()
        .find(|(name, _)| name == field)
        .map(|(_, value)| value)
        .ok_or_else(|| TinyOneError::runtime(format!("{operation}: missing {field}")))
}

fn variant_tag(
    context: &TinyRuntimeContext,
    target: &Value,
    type_name: &str,
    operation: &str,
) -> Result<i64> {
    let Value::Int(tag) = variant_field(context, target, type_name, "tag", operation)? else {
        return Err(TinyOneError::runtime(format!(
            "{operation}: tag must be an integer"
        )));
    };
    Ok(*tag)
}

fn variant_payload(
    context: &TinyRuntimeContext,
    target: &Value,
    type_name: &str,
    operation: &str,
) -> Result<Value> {
    Ok(variant_field(context, target, type_name, "payload", operation)?.clone())
}

pub fn b_result_is_ok(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::Int(
        (variant_tag(context, target, "tinyone.result.Result", "result_is_ok")? == VARIANT_OK)
            as i64,
    ))
}

pub fn b_result_is_err(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::Int(
        (variant_tag(context, target, "tinyone.result.Result", "result_is_err")? == VARIANT_ERR)
            as i64,
    ))
}

pub fn b_result_unwrap(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let tag = variant_tag(context, target, "tinyone.result.Result", "result_unwrap")?;
    if tag != VARIANT_OK {
        return Err(TinyOneError::runtime("result_unwrap: called on Err"));
    }
    variant_payload(context, target, "tinyone.result.Result", "result_unwrap")
}

pub fn b_result_unwrap_err(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let tag = variant_tag(
        context,
        target,
        "tinyone.result.Result",
        "result_unwrap_err",
    )?;
    if tag != VARIANT_ERR {
        return Err(TinyOneError::runtime("result_unwrap_err: called on Ok"));
    }
    variant_payload(
        context,
        target,
        "tinyone.result.Result",
        "result_unwrap_err",
    )
}

pub fn b_option_some(context: &mut TinyRuntimeContext, payload: Value) -> Result<Value> {
    Ok(Value::Heap(context.heap.alloc_struct(
        "tinyone.option.Option",
        vec![
            ("tag".to_string(), Value::Int(VARIANT_SOME)),
            ("payload".to_string(), payload),
        ],
    )?))
}

pub fn b_option_none(context: &mut TinyRuntimeContext) -> Result<Value> {
    Ok(Value::Heap(context.heap.alloc_struct(
        "tinyone.option.Option",
        vec![
            ("tag".to_string(), Value::Int(VARIANT_NONE)),
            ("payload".to_string(), Value::Int(0)),
        ],
    )?))
}

pub fn b_option_is_some(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::Int(
        (variant_tag(context, target, "tinyone.option.Option", "option_is_some")? == VARIANT_SOME)
            as i64,
    ))
}

pub fn b_option_is_none(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::Int(
        (variant_tag(context, target, "tinyone.option.Option", "option_is_none")? == VARIANT_NONE)
            as i64,
    ))
}

pub fn b_option_unwrap(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let tag = variant_tag(context, target, "tinyone.option.Option", "option_unwrap")?;
    if tag != VARIANT_SOME {
        return Err(TinyOneError::runtime("option_unwrap: called on None"));
    }
    variant_payload(context, target, "tinyone.option.Option", "option_unwrap")
}

// ---------------------------------------------------------------------------
// System introspection: deterministic args/env.
// ---------------------------------------------------------------------------

pub fn b_sys_argc(context: &TinyRuntimeContext) -> Result<Value> {
    Ok(Value::Int(context.sys_args.len() as i64))
}

pub fn b_sys_argv(context: &mut TinyRuntimeContext, index: &Value) -> Result<Value> {
    let index = expect_int(index, "sys_argv")?;
    let Ok(index) = usize::try_from(index) else {
        return Err(TinyOneError::runtime("sys_argv: index out of range"));
    };
    if index >= context.sys_args.len() {
        return Err(TinyOneError::runtime("sys_argv: index out of range"));
    }
    let text = context
        .sys_args
        .get(index)
        .cloned()
        .ok_or_else(|| TinyOneError::runtime("sys_argv: index out of range"))?;
    Ok(Value::Heap(context.heap.alloc_string(text)?))
}

pub fn b_sys_env_has(context: &TinyRuntimeContext, name: &Value) -> Result<Value> {
    let key = expect_string(context, name, "sys_env_has")?;
    Ok(Value::Int(context.sys_env.contains_key(&key) as i64))
}

pub fn b_sys_env_get(context: &mut TinyRuntimeContext, name: &Value) -> Result<Value> {
    let key = expect_string(context, name, "sys_env_get")?;
    let value =
        context.sys_env.get(&key).cloned().ok_or_else(|| {
            TinyOneError::runtime(format!("sys_env_get: missing variable {key:?}"))
        })?;
    Ok(Value::Heap(context.heap.alloc_string(value)?))
}

// ---------------------------------------------------------------------------
// Path & FS (Linux-first, deterministic).
//
// FS ops require unsafe at the call site because they touch host resources.
// ---------------------------------------------------------------------------

pub fn b_path_join(context: &mut TinyRuntimeContext, left: &Value, right: &Value) -> Result<Value> {
    let left = expect_string(context, left, "path_join")?;
    let right = expect_string(context, right, "path_join")?;
    let joined = if right.starts_with('/') || left.is_empty() {
        right
    } else if left.ends_with('/') {
        format!("{left}{right}")
    } else {
        format!("{left}/{right}")
    };
    Ok(Value::Heap(context.heap.alloc_string(joined)?))
}

pub fn b_path_basename(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "path_basename")?;
    let base = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Value::Heap(context.heap.alloc_string(base)?))
}

pub fn b_path_dirname(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "path_dirname")?;
    let dir = std::path::Path::new(&path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Value::Heap(context.heap.alloc_string(dir)?))
}

pub fn b_fs_read(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "fs_read")?;
    let meta = std::fs::metadata(&path)
        .map_err(|error| TinyOneError::runtime(format!("fs_read: {error}")))?;
    if meta.len() > crate::MAX_BUFFER_BYTES as u64 {
        return Err(TinyOneError::runtime(format!(
            "fs_read: file size {} exceeds limit {}",
            meta.len(),
            crate::MAX_BUFFER_BYTES
        )));
    }
    let mut file =
        File::open(&path).map_err(|error| TinyOneError::runtime(format!("fs_read: {error}")))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((crate::MAX_BUFFER_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| TinyOneError::runtime(format!("fs_read: {error}")))?;
    if bytes.len() > crate::MAX_BUFFER_BYTES {
        return Err(TinyOneError::runtime(format!(
            "fs_read: file size {} exceeds limit {}",
            bytes.len(),
            crate::MAX_BUFFER_BYTES
        )));
    }
    Ok(Value::Heap(context.heap.alloc_buffer_with(bytes)?))
}

pub fn b_fs_write(
    context: &mut TinyRuntimeContext,
    target: &Value,
    buffer: &Value,
) -> Result<Value> {
    let path = expect_string(context, target, "fs_write")?;
    let bytes = {
        let object = context.heap.get(buffer)?;
        let HeapData::Buffer(bytes) = &object.data else {
            return Err(TinyOneError::runtime("fs_write expects a buffer payload"));
        };
        bytes.clone()
    };
    std::fs::write(&path, &bytes)
        .map_err(|error| TinyOneError::runtime(format!("fs_write: {error}")))?;
    Ok(Value::Int(bytes.len() as i64))
}

pub fn b_fs_exists(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "fs_exists")?;
    Ok(Value::Int(std::path::Path::new(&path).exists() as i64))
}

pub fn b_fs_list_dir(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "fs_list_dir")?;
    let mut sorted = BTreeMap::new();
    let mut name_bytes = 0usize;
    let entries = std::fs::read_dir(&path)
        .map_err(|error| TinyOneError::runtime(format!("fs_list_dir: {error}")))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| TinyOneError::runtime(format!("fs_list_dir: {error}")))?;
        if sorted.len() >= MAX_FS_LIST_DIR_ENTRIES {
            return Err(TinyOneError::runtime(format!(
                "fs_list_dir: directory entry count exceeds limit {MAX_FS_LIST_DIR_ENTRIES}"
            )));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        name_bytes = name_bytes
            .checked_add(name.len())
            .ok_or_else(|| TinyOneError::runtime("fs_list_dir: directory name budget overflow"))?;
        if name_bytes > crate::MAX_BUFFER_BYTES {
            return Err(TinyOneError::runtime(format!(
                "fs_list_dir: directory name bytes exceed limit {}",
                crate::MAX_BUFFER_BYTES
            )));
        }
        sorted.insert(name, ());
    }
    let mut names: Vec<Value> = Vec::with_capacity(sorted.len());
    for name in sorted.into_keys() {
        names.push(Value::Heap(context.heap.alloc_string(name)?));
    }
    Ok(Value::Heap(context.heap.alloc_array(names)?))
}

// ---------------------------------------------------------------------------
// Math / Logic constants and helpers.
// ---------------------------------------------------------------------------

pub const MATH_PI_THOUSANDTHS: i64 = 3142;
pub const MATH_E_THOUSANDTHS: i64 = 2718;
pub const MATH_TAU_THOUSANDTHS: i64 = 6283;
pub const MATH_MAX_I64: i64 = i64::MAX;
pub const MATH_MIN_I64: i64 = i64::MIN;

pub fn math_constant_lookup(name: &str) -> Option<i64> {
    match name {
        "PI_THOUSANDTHS" => Some(MATH_PI_THOUSANDTHS),
        "E_THOUSANDTHS" => Some(MATH_E_THOUSANDTHS),
        "TAU_THOUSANDTHS" => Some(MATH_TAU_THOUSANDTHS),
        "MAX_I64" => Some(MATH_MAX_I64),
        "MIN_I64" => Some(MATH_MIN_I64),
        _ => None,
    }
}

pub fn b_math_const(context: &TinyRuntimeContext, name: &Value) -> Result<Value> {
    let key = expect_string(context, name, "math_const")?;
    let value = math_constant_lookup(&key)
        .ok_or_else(|| TinyOneError::runtime(format!("math_const: unknown constant {key:?}")))?;
    Ok(Value::Int(value))
}

pub fn b_math_abs(value: &Value) -> Result<Value> {
    let v = expect_int(value, "math_abs")?;
    let result = v
        .checked_abs()
        .ok_or_else(|| TinyOneError::runtime("Runtime.Memory_Overflow: math_abs"))?;
    Ok(Value::Int(result))
}

pub fn b_math_min(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "math_min")?;
    let b = expect_int(rhs, "math_min")?;
    Ok(Value::Int(a.min(b)))
}

pub fn b_math_max(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "math_max")?;
    let b = expect_int(rhs, "math_max")?;
    Ok(Value::Int(a.max(b)))
}

pub fn b_logic_and(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "logic_and")?;
    let b = expect_int(rhs, "logic_and")?;
    Ok(Value::Int(((a != 0) && (b != 0)) as i64))
}

pub fn b_logic_or(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "logic_or")?;
    let b = expect_int(rhs, "logic_or")?;
    Ok(Value::Int(((a != 0) || (b != 0)) as i64))
}

pub fn b_logic_not(value: &Value) -> Result<Value> {
    let v = expect_int(value, "logic_not")?;
    Ok(Value::Int((v == 0) as i64))
}

pub fn b_logic_xor(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "logic_xor")?;
    let b = expect_int(rhs, "logic_xor")?;
    Ok(Value::Int(((a != 0) ^ (b != 0)) as i64))
}

// ---------------------------------------------------------------------------
// Typed integer ops: widths enforced per typing_system.md.
// ---------------------------------------------------------------------------

pub fn b_type_of(context: &mut TinyRuntimeContext, value: &Value) -> Result<Value> {
    let name = match value {
        Value::Int(_) | Value::U8(_) | Value::U16(_) | Value::U32(_) => {
            runtime_integer_type_name(value).unwrap_or(TypeKind::I64.name())
        }
        Value::Pointer(p) if p.kind == "null" && p.address == 0 => TypeKind::Null.name(),
        Value::Pointer(_) => TypeKind::Pointer.name(),
        Value::Heap(_) => {
            let object = context.heap.get(value)?;
            match &object.data {
                HeapData::String(_) => TypeKind::String.name(),
                HeapData::Array(_) => TypeKind::Vec.name(),
                HeapData::Buffer(_) => TypeKind::Buffer.name(),
                HeapData::Struct(_) => {
                    if object.type_name == "tinyone.result.Result" {
                        TypeKind::Result.name()
                    } else if object.type_name == "tinyone.option.Option" {
                        TypeKind::Option.name()
                    } else if object.type_name == "tinyone.sync.Mutex" {
                        TypeKind::Mutex.name()
                    } else if object.type_name == "tinyone.sync.Atomic" {
                        TypeKind::Atomic.name()
                    } else {
                        TypeKind::Struct.name()
                    }
                }
                HeapData::Map(_) => TypeKind::Map.name(),
                HeapData::Cell(_) => TypeKind::Alloc.name(),
                HeapData::Mutex(_) => TypeKind::Mutex.name(),
                HeapData::Atomic(_) => TypeKind::Atomic.name(),
                HeapData::Thread(_) => "thread",
            }
        }
    };
    Ok(Value::Heap(context.heap.alloc_string(name.to_string())?))
}

pub fn b_type_id(context: &mut TinyRuntimeContext, type_name: &Value) -> Result<Value> {
    let name = expect_string(context, type_name, "type_id")?;
    let kind = parse_type_name(&name, "type_id")?;
    Ok(Value::Int(kind.type_id() as i64))
}

pub fn b_smallest_fit(value: &Value, context: &mut TinyRuntimeContext) -> Result<Value> {
    let v = expect_int(value, "smallest_fit")?;
    let kind = smallest_fit_literal(v);
    Ok(Value::Heap(
        context.heap.alloc_string(kind.name().to_string())?,
    ))
}

pub fn b_promote(context: &mut TinyRuntimeContext, lhs: &Value, rhs: &Value) -> Result<Value> {
    let lhs_name = expect_string(context, lhs, "promote")?;
    let rhs_name = expect_string(context, rhs, "promote")?;
    let lhs_kind = parse_type_name(&lhs_name, "promote")?;
    let rhs_kind = parse_type_name(&rhs_name, "promote")?;
    let kind = promote_integer(lhs_kind, rhs_kind)?;
    Ok(Value::Heap(
        context.heap.alloc_string(kind.name().to_string())?,
    ))
}

pub fn b_check_int_range(
    context: &TinyRuntimeContext,
    value: &Value,
    type_name: &Value,
) -> Result<Value> {
    let v = expect_int(value, "check_int_range")?;
    let name = expect_string(context, type_name, "check_int_range")?;
    let kind = parse_type_name(&name, "check_int_range")?;
    let _ = integer_range(kind)
        .ok_or_else(|| TinyOneError::runtime(format!("{} is not an integer type", kind.name())))?;
    runtime_cast_int(&Value::Int(v), kind, "check_int_range")
}

fn typed_binary(
    context: &TinyRuntimeContext,
    lhs: &Value,
    rhs: &Value,
    type_name: &Value,
    op_name: &str,
    op: impl FnOnce(i128, i128) -> Option<i128>,
) -> Result<Value> {
    let lhs = expect_int(lhs, op_name)?;
    let rhs = expect_int(rhs, op_name)?;
    let name = expect_string(context, type_name, op_name)?;
    let kind = parse_type_name(&name, op_name)?;
    let result = op(lhs as i128, rhs as i128).ok_or_else(|| {
        TinyOneError::runtime(format!(
            "Runtime.Memory_Overflow: {op_name} intermediate overflow"
        ))
    })?;
    let value = check_integer_range(kind, result)?;
    runtime_cast_int(&Value::Int(value), kind, op_name)
}

pub fn b_typed_add(
    context: &TinyRuntimeContext,
    lhs: &Value,
    rhs: &Value,
    type_name: &Value,
) -> Result<Value> {
    typed_binary(context, lhs, rhs, type_name, "typed_add", i128::checked_add)
}

pub fn b_typed_sub(
    context: &TinyRuntimeContext,
    lhs: &Value,
    rhs: &Value,
    type_name: &Value,
) -> Result<Value> {
    typed_binary(context, lhs, rhs, type_name, "typed_sub", i128::checked_sub)
}

pub fn b_typed_mul(
    context: &TinyRuntimeContext,
    lhs: &Value,
    rhs: &Value,
    type_name: &Value,
) -> Result<Value> {
    typed_binary(context, lhs, rhs, type_name, "typed_mul", i128::checked_mul)
}

pub fn b_typed_div(
    context: &TinyRuntimeContext,
    lhs: &Value,
    rhs: &Value,
    type_name: &Value,
) -> Result<Value> {
    let lhs = expect_int(lhs, "typed_div")?;
    let rhs = expect_int(rhs, "typed_div")?;
    let name = expect_string(context, type_name, "typed_div")?;
    let kind = parse_type_name(&name, "typed_div")?;
    if rhs == 0 {
        return Err(TinyOneError::runtime("Runtime.Division_By_Zero"));
    }
    let quotient = (lhs as i128) / (rhs as i128);
    let value = check_integer_range(kind, quotient)?;
    runtime_cast_int(&Value::Int(value), kind, "typed_div")
}

pub fn b_typed_neg(
    context: &TinyRuntimeContext,
    value: &Value,
    type_name: &Value,
) -> Result<Value> {
    let v = expect_int(value, "typed_neg")?;
    let name = expect_string(context, type_name, "typed_neg")?;
    let kind = parse_type_name(&name, "typed_neg")?;
    if !kind.is_signed() {
        return Err(TinyOneError::runtime(format!(
            "typed_neg: {} is not signed",
            kind.name()
        )));
    }
    let negated = (v as i128).checked_neg().ok_or_else(|| {
        TinyOneError::runtime("Runtime.Memory_Overflow: typed_neg intermediate overflow")
    })?;
    let result = check_integer_range(kind, negated)?;
    runtime_cast_int(&Value::Int(result), kind, "typed_neg")
}

pub fn b_assert(
    value: &Value,
    message: Option<&Value>,
    context: &TinyRuntimeContext,
) -> Result<Value> {
    let v = expect_int(value, "assert")?;
    if v == 0 {
        let detail = if let Some(message) = message {
            expect_string(context, message, "assert")?
        } else {
            "assertion failed".to_string()
        };
        return Err(TinyOneError::runtime(format!("Assertion failed: {detail}")));
    }
    Ok(Value::Int(1))
}
