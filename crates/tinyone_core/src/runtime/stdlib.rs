//! Stdlib bridge layer.
//!
//! All builtins added after the initial 35 are dispatched from
//! [`runtime_call_stdlib_builtin`]. They are bytecode-stable: their
//! definitions live in [`crate::builtins::BUILTINS`] after index 34.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::runtime::heap::{MapData, MapKey};
use crate::runtime::sync::{TinyMutex, TinyThreadHandle};
use crate::runtime::typing::{TypeKind, integer_range, promote_integer, smallest_fit_literal};
use crate::{
    HeapData,
    Result,
    TinyHeap,
    TinyMemory,
    TinyOneError,
    TinyRuntimeContext,
    VALUE_BYTES,
    VM,
    Value,
    expect_int,
    expect_string,
    integer_value_from_kind,
    round_to_kind,
    runtime_cast_int,
    runtime_integer_kind,
    runtime_integer_value,
};

const MAX_FS_LIST_DIR_ENTRIES: usize = 65_536;

fn expect_kind(value: &Value, kind: &str, operation: &str) -> Result<i64> {
    if runtime_integer_kind(value).is_none() {
        return Err(TinyOneError::runtime(format!("{operation} expects {kind}")));
    }
    expect_int(value, operation)
}

fn parse_type_name(text: &str, operation: &str) -> Result<TypeKind> {
    TypeKind::from_name(text).ok_or_else(|| TinyOneError::runtime(format!("{operation} unknown type name {:?}", text)))
}

fn runtime_integer_type_name(value: &Value) -> Option<&'static str> {
    runtime_integer_kind(value).map(TypeKind::name)
}

pub fn b_int_cast(value: &Value, kind: TypeKind, operation: &str) -> Result<Value> {
    runtime_cast_int(value, kind, operation)
}

/// Casts `value` to a `Value::Float` of the given float `kind`, rounding to
/// that format's precision (`round_to_kind`). Integer operands are promoted
/// to `f64` first. This is the only way to produce a non-`Fp64` float in
/// TinyLang source — float literals are always `Fp64` (see `Op::PushFloat`).
pub fn b_float_cast(value: &Value, kind: TypeKind, operation: &str) -> Result<Value> {
    let bits = match value {
        Value::Float { bits, .. } => *bits,
        _ => runtime_integer_value(value, operation)? as f64,
    };
    Ok(Value::Float {
        kind,
        bits: round_to_kind(bits, kind),
    })
}

// ---------------------------------------------------------------------------
// Vec helpers (vec_new, vec_push, vec_pop, vec_get, vec_set, vec_len)
//
// A TinyOne Vec is a heap-array used through the existing array machinery
// but accessed via length-aware safe builtins. Existing `array`, `push`,
// `pop`, `len`, and indexing builtins continue to work without modification.
// ---------------------------------------------------------------------------

pub fn b_vec_new(context: &mut TinyRuntimeContext) -> Result<Value> {
    Ok(Value::Heap(context.heap().alloc_array(Vec::new())?))
}

pub fn b_vec_clear(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let mut heap = context.heap();
    let cleared = {
        let object = heap.get_mut(target)?;
        let HeapData::Array(values) = &mut object.data else {
            return Err(TinyOneError::runtime("vec_clear expects a vec/array"));
        };
        let cleared = values.len();
        values.clear();
        cleared
    };
    heap.record_shrink(cleared.saturating_mul(VALUE_BYTES))?;
    Ok(Value::I64(0))
}

// ---------------------------------------------------------------------------
// Map helpers (map_new, map_set, map_get, map_has, map_del, map_len, map_keys)
//
// Map entries remain insertion-ordered in Ralloc storage, with a canonical
// host-side index used only to locate those encoded entries.
// ---------------------------------------------------------------------------

pub fn b_map_new(context: &mut TinyRuntimeContext) -> Result<Value> {
    Ok(Value::Heap(context.heap().alloc_map(Vec::new())?))
}

pub fn b_map_set(context: &mut TinyRuntimeContext, target: &Value, key: Value, value: Value) -> Result<Value> {
    // Keep canonicalization and mutation in one heap-lock window. Pointer-key
    // validation is part of the language operation, not a preparatory query.
    let encoded_key = crate::runtime::value_codec::encode_value(&key)?;
    let encoded_value = crate::runtime::value_codec::encode_value(&value)?;
    let mut heap = context.heap();
    let lookup_key = canonical_map_key(&heap, &key)?;
    b_map_set_encoded(&mut heap, target, lookup_key, encoded_key, encoded_value, value)
}

/// JIT fast path for the overwhelmingly common counted-loop map key. It
/// preserves the generic path for every other key shape (including pointers)
/// while avoiding dynamic numeric coercion on every insert/update.
pub(crate) fn b_map_set_i64(context: &mut TinyRuntimeContext, target: &Value, key: i64, value: Value) -> Result<Value> {
    let encoded_key = crate::runtime::value_codec::encode_i64(key);
    let encoded_value = crate::runtime::value_codec::encode_value(&value)?;
    let mut heap = context.heap();
    b_map_set_encoded(&mut heap, target, Some(MapKey::Integer(i128::from(key))), encoded_key, encoded_value, value)
}

fn b_map_set_encoded(
    heap: &mut TinyHeap,
    target: &Value,
    lookup_key: Option<MapKey>,
    encoded_key: [u8; crate::runtime::value_codec::ENCODED_VALUE_BYTES],
    encoded_value: [u8; crate::runtime::value_codec::ENCODED_VALUE_BYTES],
    value: Value,
) -> Result<Value> {
    let mut inserted = false;
    let address = match target {
        Value::Heap(reference) => reference.address,
        _ => return Err(TinyOneError::runtime("Expected heap pointer")),
    };
    let existing = {
        let object = heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_set expects a map"));
        };
        indexed_map_index(heap, entries, lookup_key.as_ref())?
    };
    if existing.is_none() {
        heap.ensure_can_allocate_delta(VALUE_BYTES.saturating_mul(2))?;
    }
    {
        // `heap.get(target)` validated the generation and this is an exclusive
        // heap borrow, so the live occupant cannot change before this mutation.
        let object = heap
            .objects
            .get_mut(address)
            .and_then(Option::as_mut)
            .expect("validated map slot remains live");
        let HeapData::Map(entries) = &mut object.data else {
            return Err(TinyOneError::runtime("map_set expects a map"));
        };
        if let Some(idx) = existing {
            let slot = entries
                .get_mut(idx)
                .ok_or_else(|| TinyOneError::runtime("map_set: internal index error"))?;
            slot[crate::runtime::value_codec::ENCODED_VALUE_BYTES..].copy_from_slice(&encoded_value);
        } else {
            let index = entries.len();
            let mut pair = [0u8; 2 * crate::runtime::value_codec::ENCODED_VALUE_BYTES];
            let width = crate::runtime::value_codec::ENCODED_VALUE_BYTES;
            pair[..width].copy_from_slice(&encoded_key);
            pair[width..].copy_from_slice(&encoded_value);
            entries.push(&pair)?;
            if matches!(&lookup_key, Some(MapKey::Pointer { .. })) {
                entries.pointer_indices.push(index);
            }
            if let Some(lookup_key) = lookup_key {
                entries.index.insert(lookup_key, index);
            }
            inserted = true;
        }
    }
    if inserted {
        heap.record_growth(VALUE_BYTES.saturating_mul(2))?;
    }
    Ok(value)
}

pub fn b_map_get(context: &mut TinyRuntimeContext, target: &Value, key: &Value) -> Result<Value> {
    let heap = context.heap();
    let lookup_key = canonical_map_key(&heap, key)?;
    let object = heap.get(target)?;
    let HeapData::Map(entries) = &object.data else {
        return Err(TinyOneError::runtime("map_get expects a map"));
    };
    let index = indexed_map_index(&heap, entries, lookup_key.as_ref())?
        .ok_or_else(|| TinyOneError::runtime("map_get: missing key"))?;
    crate::runtime::heap::decode_map_value(entries, index)
        .ok_or_else(|| TinyOneError::runtime("map_get: internal index error"))
}

/// Integer-key counterpart of [`b_map_get`] used by direct JIT builtin calls.
/// Pointer-sidecar validation remains in `indexed_map_index`, so maps that
/// contain pointer keys retain stale-pointer rejection regardless of lookup
/// key type.
pub(crate) fn b_map_get_i64(context: &mut TinyRuntimeContext, target: &Value, key: i64) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Map(entries) = &object.data else {
        return Err(TinyOneError::runtime("map_get expects a map"));
    };
    let lookup_key = MapKey::Integer(i128::from(key));
    let index = indexed_map_index(&heap, entries, Some(&lookup_key))?
        .ok_or_else(|| TinyOneError::runtime("map_get: missing key"))?;
    crate::runtime::heap::decode_map_value(entries, index)
        .ok_or_else(|| TinyOneError::runtime("map_get: internal index error"))
}

pub fn b_map_has(context: &TinyRuntimeContext, target: &Value, key: &Value) -> Result<Value> {
    let heap = context.heap();
    let lookup_key = canonical_map_key(&heap, key)?;
    let object = heap.get(target)?;
    let HeapData::Map(entries) = &object.data else {
        return Err(TinyOneError::runtime("map_has expects a map"));
    };
    Ok(Value::I64(indexed_map_index(&heap, entries, lookup_key.as_ref())?.is_some() as i64))
}

pub fn b_map_del(context: &mut TinyRuntimeContext, target: &Value, key: &Value) -> Result<Value> {
    let mut heap = context.heap();
    let lookup_key = canonical_map_key(&heap, key)?;
    let to_remove = {
        let object = heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_del expects a map"));
        };
        indexed_map_index(&heap, entries, lookup_key.as_ref())?
    };
    let removed = if let Some(idx) = to_remove {
        {
            let object = heap.get_mut(target)?;
            let HeapData::Map(entries) = &mut object.data else {
                return Err(TinyOneError::runtime("map_del expects a map"));
            };
            entries.remove(idx);
            entries.remove_index(idx);
        }
        heap.record_shrink(VALUE_BYTES.saturating_mul(2))?;
        true
    } else {
        false
    };
    if removed { Ok(Value::I64(1)) } else { Ok(Value::I64(0)) }
}

fn validate_map_pointer_base(
    heap: &TinyHeap,
    address: usize,
    generation: u64,
    kind: crate::runtime::value::PointerKind,
) -> Result<()> {
    if kind == crate::runtime::value::PointerKind::Null && address == 0 {
        return Ok(());
    }
    match kind {
        crate::runtime::value::PointerKind::Object
        | crate::runtime::value::PointerKind::Array
        | crate::runtime::value::PointerKind::Buffer
        | crate::runtime::value::PointerKind::Field => {
            heap.get_address(address, generation)?;
            Ok(())
        }
        crate::runtime::value::PointerKind::Null => {
            Err(TinyOneError::runtime(format!("map key got unknown raw pointer kind {kind:?}")))
        }
    }
}

fn canonical_map_key(heap: &TinyHeap, value: &Value) -> Result<Option<MapKey>> {
    match value {
        Value::I64(_) | Value::U8(_) | Value::U16(_) | Value::U32(_) => {
            Ok(Some(MapKey::Integer(runtime_integer_value(value, "map key")?)))
        }
        Value::Pointer(pointer) => {
            validate_map_pointer_base(heap, pointer.address, pointer.generation, pointer.kind)?;
            Ok(Some(MapKey::Pointer {
                address:    pointer.address,
                kind:       pointer.kind,
                index:      pointer.index,
                field:      pointer.field.clone(),
                generation: pointer.generation,
            }))
        }
        Value::Heap(reference) => {
            let Ok(object) = heap.get(value) else {
                return Ok(None);
            };
            match &object.data {
                HeapData::String(text) => Ok(Some(MapKey::String(crate::runtime::heap::heap_str(text)?.to_owned()))),
                _ => {
                    Ok(Some(MapKey::HeapObject {
                        address:    reference.address,
                        generation: reference.generation,
                    }))
                }
            }
        }
        _ => Ok(None),
    }
}

fn indexed_map_index(heap: &TinyHeap, entries: &MapData, lookup_key: Option<&MapKey>) -> Result<Option<usize>> {
    // Preserve the contract that any stale pointer key invalidates the map
    // operation. Reading only its encoded base avoids allocating the stored
    // field name, and validation shares this operation's existing heap lock.
    for index in &entries.pointer_indices {
        let encoded = crate::runtime::heap::encoded_map_key(entries, *index)
            .ok_or_else(|| TinyOneError::runtime("map: internal pointer index error"))?;
        let (address, generation, kind) = crate::runtime::value_codec::encoded_pointer_base(encoded)
            .ok_or_else(|| TinyOneError::runtime("map: invalid pointer sidecar entry"))?;
        validate_map_pointer_base(heap, address, generation, kind)?;
    }

    Ok(lookup_key.and_then(|lookup_key| entries.index.get(lookup_key).copied()))
}

pub fn b_map_len(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Map(entries) = &object.data else {
        return Err(TinyOneError::runtime("map_len expects a map"));
    };
    Ok(Value::I64(entries.len() as i64))
}

pub fn b_map_keys(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let keys: Vec<Value> = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_keys expects a map"));
        };
        crate::runtime::heap::decode_map_entries(entries)
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    };
    Ok(Value::Heap(context.heap().alloc_array(keys)?))
}

pub fn b_map_values(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let values: Vec<Value> = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Map(entries) = &object.data else {
            return Err(TinyOneError::runtime("map_values expects a map"));
        };
        crate::runtime::heap::decode_map_entries(entries)
            .into_iter()
            .map(|(_, v)| v)
            .collect()
    };
    Ok(Value::Heap(context.heap().alloc_array(values)?))
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
    Value::I64(IO_FD_STDOUT)
}
pub fn b_io_stderr() -> Value {
    Value::I64(IO_FD_STDERR)
}
pub fn b_io_stdin() -> Value {
    Value::I64(IO_FD_STDIN)
}

pub fn b_io_write(context: &mut TinyRuntimeContext, fd: &Value, text_value: &Value) -> Result<Value> {
    let fd = expect_kind(fd, "an integer file descriptor", "io_write")?;
    let text = expect_string(context, text_value, "io_write")?;
    let bytes = text.len() as i64;
    match fd {
        IO_FD_STDOUT => context.io_stdout.push_str(&text),
        IO_FD_STDERR => context.io_stderr.push_str(&text),
        IO_FD_STDIN => {
            return Err(TinyOneError::runtime("io_write: cannot write to stdin (fd 0)"));
        }
        _ => {
            return Err(TinyOneError::runtime(format!("io_write: unsupported fd {fd}")));
        }
    }
    Ok(Value::I64(bytes))
}

pub fn b_io_writeln(context: &mut TinyRuntimeContext, fd: &Value, text_value: &Value) -> Result<Value> {
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
            return Err(TinyOneError::runtime("io_writeln: cannot write to stdin (fd 0)"));
        }
        _ => {
            return Err(TinyOneError::runtime(format!("io_writeln: unsupported fd {fd}")));
        }
    }
    Ok(Value::I64(bytes))
}

pub fn b_io_read_line(context: &mut TinyRuntimeContext) -> Result<Value> {
    let raw = context.read_raw()?;
    Ok(Value::Heap(context.heap().alloc_string(raw)?))
}

pub fn b_io_flush(_context: &mut TinyRuntimeContext, _fd: &Value) -> Result<Value> {
    // No-op for deterministic test doubles. Flushing the real stdout still
    // happens through the host once `VM::run` returns.
    Ok(Value::I64(0))
}

pub fn b_io_capture_stdout(context: &mut TinyRuntimeContext) -> Result<Value> {
    let text = std::mem::take(&mut context.io_stdout);
    Ok(Value::Heap(context.heap().alloc_string(text)?))
}

pub fn b_io_capture_stderr(context: &mut TinyRuntimeContext) -> Result<Value> {
    let text = std::mem::take(&mut context.io_stderr);
    Ok(Value::Heap(context.heap().alloc_string(text)?))
}

// ---------------------------------------------------------------------------
// String & Unicode helpers.
// ---------------------------------------------------------------------------

pub fn b_str_byte_len(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let text = expect_string(context, target, "str_byte_len")?;
    Ok(Value::I64(text.len() as i64))
}

pub fn b_str_char_len(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let text = expect_string(context, target, "str_char_len")?;
    Ok(Value::I64(text.chars().count() as i64))
}

pub fn b_str_byte_at(context: &TinyRuntimeContext, target: &Value, index: &Value) -> Result<Value> {
    let text = expect_string(context, target, "str_byte_at")?;
    let index = expect_int(index, "str_byte_at")?;
    if index < 0 {
        return Err(TinyOneError::runtime("str_byte_at: negative index"));
    }
    let index = usize::try_from(index).map_err(|_| TinyOneError::runtime("str_byte_at: index is too large"))?;
    let bytes = text.as_bytes();
    if index >= bytes.len() {
        return Err(TinyOneError::runtime("str_byte_at: index out of bounds"));
    }
    let byte = bytes
        .get(index)
        .ok_or_else(|| TinyOneError::runtime("str_byte_at: index out of bounds"))?;
    Ok(Value::I64(*byte as i64))
}

pub fn b_str_char_at(context: &mut TinyRuntimeContext, target: &Value, index: &Value) -> Result<Value> {
    let text = expect_string(context, target, "str_char_at")?;
    let index = expect_int(index, "str_char_at")?;
    if index < 0 {
        return Err(TinyOneError::runtime("str_char_at: negative index"));
    }
    let index = usize::try_from(index).map_err(|_| TinyOneError::runtime("str_char_at: index is too large"))?;
    let ch = text
        .chars()
        .nth(index)
        .ok_or_else(|| TinyOneError::runtime("str_char_at: index out of bounds"))?;
    Ok(Value::Heap(context.heap().alloc_string(ch.to_string())?))
}

pub fn b_str_slice(context: &mut TinyRuntimeContext, target: &Value, start: &Value, end: &Value) -> Result<Value> {
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
    let total_chars =
        i64::try_from(text.chars().count()).map_err(|_| TinyOneError::runtime("str_slice: string is too large"))?;
    if start > total_chars || end > total_chars {
        return Err(TinyOneError::runtime("str_slice: bound out of range"));
    }
    let char_byte_offset = |target: i64| -> Result<usize> {
        if target == total_chars {
            Ok(text_bytes)
        } else {
            let target = usize::try_from(target).map_err(|_| TinyOneError::runtime("str_slice: bound is too large"))?;
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
    Ok(Value::Heap(context.heap().alloc_string(sliced)?))
}

pub fn b_str_concat(context: &mut TinyRuntimeContext, left: &Value, right: &Value) -> Result<Value> {
    let mut left = expect_string(context, left, "str_concat")?;
    let right = expect_string(context, right, "str_concat")?;
    left.push_str(&right);
    Ok(Value::Heap(context.heap().alloc_string(left)?))
}

pub fn b_str_is_utf8(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    // String values in TinyOne are always UTF-8 by construction. Probe a
    // buffer instead so external bytes can be validated before becoming a
    // String. If the target is a buffer we run std::str::from_utf8 on its
    // bytes.
    if let Ok(text) = expect_string(context, target, "str_is_utf8") {
        let _ = text;
        return Ok(Value::I64(1));
    }
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Buffer(bytes) = &object.data else {
        return Err(TinyOneError::runtime("str_is_utf8 expects a String or Buffer"));
    };
    Ok(Value::I64(std::str::from_utf8(bytes.as_slice()).is_ok() as i64))
}

pub fn b_str_from_buffer(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let bytes = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Buffer(bytes) = &object.data else {
            return Err(TinyOneError::runtime("str_from_buffer expects a Buffer"));
        };
        bytes.as_slice().to_vec()
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| TinyOneError::runtime("str_from_buffer: invalid UTF-8"))?
        .to_string();
    Ok(Value::Heap(context.heap().alloc_string(text)?))
}

// ---------------------------------------------------------------------------
// Threading & sync (single-thread semantic shells).
//
// `Mutex` and `Atomic` are heap-allocated cells with a small protocol. They
// keep the VM honest about misuse (double-lock, unlock-when-unlocked) so
// programs validate the same way on both runtimes.
// ---------------------------------------------------------------------------

pub fn b_mutex_new(context: &mut TinyRuntimeContext) -> Result<Value> {
    let m = TinyMutex::new();
    Ok(Value::Heap(context.heap().alloc_mutex(m)?))
}

/// Acquires the mutex. MUST release the heap lock before blocking — otherwise
/// the calling thread holds the heap Mutex while waiting on TinyMutex, which
/// would deadlock any other thread trying to allocate or access heap objects.
pub fn b_mutex_lock(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    // Step 1: extract the Arc<TinyMutex> — releases the heap guard.
    let mutex_arc = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Mutex(m) = &object.data else {
            return Err(TinyOneError::runtime("mutex_lock expects a Mutex"));
        };
        Arc::clone(m)
        // heap guard drops here — heap lock released before we block
    };
    mutex_arc.lock()?;
    Ok(Value::I64(1))
}

pub fn b_mutex_unlock(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let mutex_arc = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Mutex(m) = &object.data else {
            return Err(TinyOneError::runtime("mutex_unlock expects a Mutex"));
        };
        Arc::clone(m)
    };
    mutex_arc.unlock()?;
    Ok(Value::I64(0))
}

pub fn b_atomic_new(context: &mut TinyRuntimeContext, init: &Value) -> Result<Value> {
    let init = expect_int(init, "atomic_new")?;
    Ok(Value::Heap(context.heap().alloc_atomic(init)?))
}

pub fn b_atomic_load(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let atomic_arc = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Atomic(a) = &object.data else {
            return Err(TinyOneError::runtime("atomic_load expects an Atomic"));
        };
        Arc::clone(a)
    };
    Ok(Value::I64(atomic_arc.load(Ordering::SeqCst)))
}

pub fn b_atomic_store(context: &TinyRuntimeContext, target: &Value, new_value: &Value) -> Result<Value> {
    let new_val = expect_int(new_value, "atomic_store")?;
    let atomic_arc = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Atomic(a) = &object.data else {
            return Err(TinyOneError::runtime("atomic_store expects an Atomic"));
        };
        Arc::clone(a)
    };
    atomic_arc.store(new_val, Ordering::SeqCst);
    Ok(Value::I64(new_val))
}

pub fn b_atomic_add(context: &TinyRuntimeContext, target: &Value, delta: &Value) -> Result<Value> {
    let delta_val = expect_int(delta, "atomic_add")?;
    let atomic_arc = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Atomic(a) = &object.data else {
            return Err(TinyOneError::runtime("atomic_add expects an Atomic"));
        };
        Arc::clone(a)
    };
    // CAS loop: check for overflow before mutating, retry on concurrent contention.
    loop {
        let current = atomic_arc.load(Ordering::SeqCst);
        let next = current
            .checked_add(delta_val)
            .ok_or_else(|| TinyOneError::runtime("Runtime.Memory_Overflow: atomic_add overflow"))?;
        if atomic_arc
            .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Ok(Value::I64(next));
        }
    }
}

pub fn b_thread_spawn(
    context: &mut TinyRuntimeContext,
    global_memory: &TinyMemory,
    caller_function: Option<usize>,
    args: &[Value],
) -> Result<Value> {
    let fn_name = {
        let heap = context.heap();
        let obj = heap.get(&args[0])?;
        let HeapData::String(s) = &obj.data else {
            return Err(TinyOneError::runtime("thread_spawn: first argument must be a function name string"));
        };
        crate::runtime::heap::heap_str(s)?.to_owned()
    };

    let verified = context
        .verified_program
        .clone()
        .ok_or_else(|| TinyOneError::runtime("thread_spawn: runtime has no verified program"))?;
    let program_arc = verified.program_arc();

    let fn_args = args[1..].to_vec();
    let (fn_index, fn_param_count) = program_arc
        .callable_function_from(caller_function, &fn_name)
        .map(|(index, function)| (index, function.param_count))
        .ok_or_else(|| {
            TinyOneError::runtime(format!("thread_spawn: function {:?} not found or not exported", fn_name))
        })?;

    if fn_args.len() != fn_param_count {
        return Err(TinyOneError::runtime(format!(
            "thread_spawn: {:?} expects {} argument(s), got {}",
            fn_name,
            fn_param_count,
            fn_args.len()
        )));
    }

    let heap_arc = Arc::clone(&context.heap_arc);
    let thread_globals = global_memory.try_clone()?;
    let sys_args = context.sys_args.clone();
    let sys_env = context.sys_env.clone();

    let handle = std::thread::spawn(move || {
        let mut thread_ctx = TinyRuntimeContext::with_heap(heap_arc);
        thread_ctx.program_arc = Some(Arc::clone(&program_arc));
        thread_ctx.verified_program = Some(verified.clone());
        thread_ctx.set_sys_args(sys_args);
        thread_ctx.set_sys_env(sys_env);
        let mut thread_stdout: Vec<u8> = Vec::new();
        let vm = VM::new_unchecked_with_context(&verified, thread_globals, thread_ctx);
        let result = vm.run_function_by_index(fn_index, fn_args, &mut thread_stdout);
        (result, thread_stdout)
    });

    let thread_handle = TinyThreadHandle::new(handle);
    Ok(Value::Heap(context.heap().alloc_thread(thread_handle)?))
}

pub fn b_thread_join(context: &mut TinyRuntimeContext, args: &[Value]) -> Result<Value> {
    let handle_arc = {
        let heap = context.heap();
        let object = heap.get(&args[0])?;
        let HeapData::Thread(h) = &object.data else {
            return Err(TinyOneError::runtime("thread_join expects a thread handle"));
        };
        Arc::clone(h)
    };
    let (value, thread_stdout) = handle_arc.join()?;
    context.queued_stdout.extend_from_slice(&thread_stdout);
    Ok(value)
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
    Ok(Value::Heap(context.heap().alloc_struct(
        "tinyone.result.Result",
        vec![
            ("tag".to_string(), Value::I64(VARIANT_OK)),
            ("payload".to_string(), payload),
        ],
    )?))
}

pub fn b_result_err(context: &mut TinyRuntimeContext, payload: Value) -> Result<Value> {
    Ok(Value::Heap(context.heap().alloc_struct(
        "tinyone.result.Result",
        vec![
            ("tag".to_string(), Value::I64(VARIANT_ERR)),
            ("payload".to_string(), payload),
        ],
    )?))
}

/// Returns the field value as an owned Value (cloned from heap).
fn variant_field(
    context: &TinyRuntimeContext,
    target: &Value,
    type_name: &str,
    field: &str,
    operation: &str,
) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    if object.type_name != type_name {
        return Err(TinyOneError::runtime(format!("{operation}: expected {type_name}, got {:?}", object.type_name)));
    }
    let HeapData::Struct(record) = &object.data else {
        return Err(TinyOneError::runtime(format!("{operation}: corrupt {type_name}")));
    };
    record
        .get(field)
        .ok_or_else(|| TinyOneError::runtime(format!("{operation}: missing {field}")))
}

fn variant_tag(context: &TinyRuntimeContext, target: &Value, type_name: &str, operation: &str) -> Result<i64> {
    let tag_value = variant_field(context, target, type_name, "tag", operation)?;
    let Value::I64(tag) = tag_value else {
        return Err(TinyOneError::runtime(format!("{operation}: tag must be an integer")));
    };
    Ok(tag)
}

fn variant_payload(context: &TinyRuntimeContext, target: &Value, type_name: &str, operation: &str) -> Result<Value> {
    variant_field(context, target, type_name, "payload", operation)
}

pub fn b_result_is_ok(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::I64(
        (variant_tag(context, target, "tinyone.result.Result", "result_is_ok")? == VARIANT_OK) as i64,
    ))
}

pub fn b_result_is_err(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::I64(
        (variant_tag(context, target, "tinyone.result.Result", "result_is_err")? == VARIANT_ERR) as i64,
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
    let tag = variant_tag(context, target, "tinyone.result.Result", "result_unwrap_err")?;
    if tag != VARIANT_ERR {
        return Err(TinyOneError::runtime("result_unwrap_err: called on Ok"));
    }
    variant_payload(context, target, "tinyone.result.Result", "result_unwrap_err")
}

pub fn b_option_some(context: &mut TinyRuntimeContext, payload: Value) -> Result<Value> {
    Ok(Value::Heap(context.heap().alloc_struct(
        "tinyone.option.Option",
        vec![
            ("tag".to_string(), Value::I64(VARIANT_SOME)),
            ("payload".to_string(), payload),
        ],
    )?))
}

pub fn b_option_none(context: &mut TinyRuntimeContext) -> Result<Value> {
    Ok(Value::Heap(context.heap().alloc_struct(
        "tinyone.option.Option",
        vec![
            ("tag".to_string(), Value::I64(VARIANT_NONE)),
            ("payload".to_string(), Value::I64(0)),
        ],
    )?))
}

pub fn b_option_is_some(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::I64(
        (variant_tag(context, target, "tinyone.option.Option", "option_is_some")? == VARIANT_SOME) as i64,
    ))
}

pub fn b_option_is_none(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    Ok(Value::I64(
        (variant_tag(context, target, "tinyone.option.Option", "option_is_none")? == VARIANT_NONE) as i64,
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
// Runtime scaffold types with language-visible representations.
//
// These values are deliberately exposed through small constructors and
// accessors rather than through Rust-only heap allocation.  The accessors
// preserve the representation's invariants and make every operation useful
// from TinyLang without requiring a static type checker first.
// ---------------------------------------------------------------------------

fn unsigned_u32(value: &Value, operation: &str) -> Result<u32> {
    let value = expect_int(value, operation)?;
    u32::try_from(value).map_err(|_| TinyOneError::runtime(format!("{operation}: expected a non-negative u32")))
}

pub fn b_closure_new(
    context: &mut TinyRuntimeContext,
    caller_function: Option<usize>,
    function_name: &Value,
    captures: &Value,
) -> Result<Value> {
    let name = expect_string(context, function_name, "closure_new")?;
    let captured_values = {
        let heap = context.heap();
        let object = heap.get(captures)?;
        let HeapData::Array(values) = &object.data else {
            return Err(TinyOneError::runtime("closure_new expects an array of captures"));
        };
        crate::runtime::heap::decode_array_values(values)
    };
    let program = context
        .program_arc
        .as_ref()
        .ok_or_else(|| TinyOneError::runtime("closure_new: runtime has no compiled program"))?;
    let function_id = program
        .callable_function_from(caller_function, &name)
        .map(|(index, _)| index)
        .ok_or_else(|| TinyOneError::runtime(format!("closure_new: function {name:?} not found or not exported")))?;
    Ok(Value::Heap(context.heap().alloc_closure(function_id as u32, captured_values)?))
}

pub fn b_closure_function(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Closure { function_id, .. } = &object.data else {
        return Err(TinyOneError::runtime("closure_function expects a Closure"));
    };
    Ok(Value::I64(*function_id as i64))
}

pub fn b_closure_captures(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let values = {
        let heap = context.heap();
        let object = heap.get(target)?;
        let HeapData::Closure { captures, .. } = &object.data else {
            return Err(TinyOneError::runtime("closure_captures expects a Closure"));
        };
        crate::runtime::heap::decode_array_values(captures)
    };
    Ok(Value::Heap(context.heap().alloc_array(values)?))
}

pub fn b_sum_new(context: &mut TinyRuntimeContext, tag: &Value, payload: Option<&Value>) -> Result<Value> {
    let tag = unsigned_u32(tag, "sum_new")?;
    Ok(Value::Heap(context.heap().alloc_sum(tag, payload.cloned())?))
}

pub fn b_sum_tag(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Sum { tag, .. } = &object.data else {
        return Err(TinyOneError::runtime("sum_tag expects a Sum"));
    };
    Ok(Value::I64(*tag as i64))
}

pub fn b_sum_has_payload(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Sum { payload, .. } = &object.data else {
        return Err(TinyOneError::runtime("sum_has_payload expects a Sum"));
    };
    Ok(Value::I64(payload.is_some() as i64))
}

pub fn b_sum_unwrap(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Sum { payload, .. } = &object.data else {
        return Err(TinyOneError::runtime("sum_unwrap expects a Sum"));
    };
    payload
        .as_ref()
        .map(crate::runtime::heap::decode_value_slot)
        .ok_or_else(|| TinyOneError::runtime("sum_unwrap: variant has no payload"))
}

pub fn b_tagged_union_new(context: &mut TinyRuntimeContext, tag: &Value, payload: &Value) -> Result<Value> {
    Ok(Value::Heap(
        context
            .heap()
            .alloc_tagged_union(unsigned_u32(tag, "tagged_union_new")?, payload.clone())?,
    ))
}

pub fn b_tagged_union_tag(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::TaggedUnion { tag, .. } = &object.data else {
        return Err(TinyOneError::runtime("tagged_union_tag expects a TaggedUnion"));
    };
    Ok(Value::I64(*tag as i64))
}

pub fn b_tagged_union_unwrap(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::TaggedUnion { payload, .. } = &object.data else {
        return Err(TinyOneError::runtime("tagged_union_unwrap expects a TaggedUnion"));
    };
    Ok(crate::runtime::heap::decode_value_slot(payload))
}

pub fn b_dyn_new(context: &mut TinyRuntimeContext, type_id: &Value, vtable_id: &Value, value: &Value) -> Result<Value> {
    let type_id = u16::try_from(unsigned_u32(type_id, "dyn_new")?)
        .map_err(|_| TinyOneError::runtime("dyn_new: type id exceeds u16"))?;
    let vtable_id = unsigned_u32(vtable_id, "dyn_new")?;
    Ok(Value::Heap(context.heap().alloc_dyn(type_id, vtable_id, value.clone())?))
}

pub fn b_dyn_metadata(context: &TinyRuntimeContext, target: &Value, want_vtable: bool) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Dyn { type_id, vtable_id, .. } = &object.data else {
        return Err(TinyOneError::runtime("dyn metadata expects a Dyn"));
    };
    Ok(Value::I64(if want_vtable {
        *vtable_id as i64
    } else {
        *type_id as i64
    }))
}

pub fn b_dyn_unwrap(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Dyn { value, .. } = &object.data else {
        return Err(TinyOneError::runtime("dyn_unwrap expects a Dyn"));
    };
    Ok(crate::runtime::heap::decode_value_slot(value))
}

pub fn b_box_new(context: &mut TinyRuntimeContext, value: &Value) -> Result<Value> {
    Ok(Value::Heap(context.heap().alloc_box(value.clone())?))
}

pub fn b_box_get(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(target)?;
    let HeapData::Box(value) = &object.data else {
        return Err(TinyOneError::runtime("box_get expects a Box"));
    };
    Ok(crate::runtime::heap::decode_value_slot(value))
}

pub fn b_box_set(context: &mut TinyRuntimeContext, target: &Value, value: &Value) -> Result<Value> {
    let mut heap = context.heap();
    let object = heap.get_mut(target)?;
    let HeapData::Box(bytes) = &mut object.data else {
        return Err(TinyOneError::runtime("box_set expects a Box"));
    };
    let encoded = crate::runtime::value_codec::encode_value(value)?;
    bytes.as_mut_slice().copy_from_slice(&encoded);
    Ok(value.clone())
}

pub fn b_char_new(context: &mut TinyRuntimeContext, value: &Value) -> Result<Value> {
    let scalar = unsigned_u32(value, "char_new")?;
    char::from_u32(scalar).ok_or_else(|| TinyOneError::runtime("char_new: invalid Unicode scalar value"))?;
    Ok(Value::Heap(context.heap().alloc_char(scalar)?))
}

pub fn b_fd_new(context: &mut TinyRuntimeContext, value: &Value) -> Result<Value> {
    let fd = i32::try_from(expect_int(value, "fd_new")?)
        .map_err(|_| TinyOneError::runtime("fd_new: descriptor exceeds i32"))?;
    Ok(Value::Heap(context.heap().alloc_file_descriptor(fd)?))
}

pub fn b_char_buffer_new(context: &mut TinyRuntimeContext, values: &Value) -> Result<Value> {
    let chars = {
        let heap = context.heap();
        let object = heap.get(values)?;
        let HeapData::Array(values) = &object.data else {
            return Err(TinyOneError::runtime("char_buffer_new expects an array"));
        };
        crate::runtime::heap::decode_array_values(values)
            .iter()
            .map(|value| unsigned_u32(value, "char_buffer_new"))
            .collect::<Result<Vec<_>>>()?
    };
    for scalar in &chars {
        char::from_u32(*scalar)
            .ok_or_else(|| TinyOneError::runtime("char_buffer_new: invalid Unicode scalar value"))?;
    }
    Ok(Value::Heap(context.heap().alloc_char_buffer(chars)?))
}

pub fn b_record_new(context: &mut TinyRuntimeContext, source: &Value) -> Result<Value> {
    let fields = {
        let heap = context.heap();
        let object = heap.get(source)?;
        match &object.data {
            HeapData::Struct(record) | HeapData::Record(record) => record.fields(),
            _ => {
                return Err(TinyOneError::runtime("record_new expects a struct or record"));
            }
        }
    };
    Ok(Value::Heap(context.heap().alloc_record(fields)?))
}

pub fn b_dictionary_new(context: &mut TinyRuntimeContext, source: &Value) -> Result<Value> {
    let entries = {
        let heap = context.heap();
        let object = heap.get(source)?;
        match &object.data {
            HeapData::Map(entries) => crate::runtime::heap::decode_map_entries(entries),
            HeapData::Dictionary(entries) => crate::runtime::heap::decode_map_entries(entries),
            _ => {
                return Err(TinyOneError::runtime("dictionary_new expects a map or dictionary"));
            }
        }
    };
    Ok(Value::Heap(context.heap().alloc_dictionary(entries)?))
}

pub fn b_alloc_new(context: &mut TinyRuntimeContext, type_name: &Value, buffer: &Value) -> Result<Value> {
    let type_name = expect_string(context, type_name, "alloc_new")?;
    let kind = parse_type_name(&type_name, "alloc_new")?;
    let bytes = {
        let heap = context.heap();
        let object = heap.get(buffer)?;
        let HeapData::Buffer(bytes) = &object.data else {
            return Err(TinyOneError::runtime("alloc_new expects a Buffer"));
        };
        bytes.as_slice().to_vec()
    };
    Ok(Value::Heap(context.heap().alloc_raw(kind, bytes)?))
}

// ---------------------------------------------------------------------------
// System introspection: deterministic args/env.
// ---------------------------------------------------------------------------

pub fn b_sys_argc(context: &TinyRuntimeContext) -> Result<Value> {
    Ok(Value::I64(context.sys_args.len() as i64))
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
    Ok(Value::Heap(context.heap().alloc_string(text)?))
}

pub fn b_sys_env_has(context: &TinyRuntimeContext, name: &Value) -> Result<Value> {
    let key = expect_string(context, name, "sys_env_has")?;
    Ok(Value::I64(context.sys_env.contains_key(&key) as i64))
}

pub fn b_sys_env_get(context: &mut TinyRuntimeContext, name: &Value) -> Result<Value> {
    let key = expect_string(context, name, "sys_env_get")?;
    let value = context
        .sys_env
        .get(&key)
        .cloned()
        .ok_or_else(|| TinyOneError::runtime(format!("sys_env_get: missing variable {key:?}")))?;
    Ok(Value::Heap(context.heap().alloc_string(value)?))
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
    Ok(Value::Heap(context.heap().alloc_string(joined)?))
}

pub fn b_path_basename(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "path_basename")?;
    let base = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Value::Heap(context.heap().alloc_string(base)?))
}

pub fn b_path_dirname(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "path_dirname")?;
    let dir = std::path::Path::new(&path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Value::Heap(context.heap().alloc_string(dir)?))
}

pub fn b_fs_read(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "fs_read")?;
    let meta = std::fs::metadata(&path).map_err(|error| TinyOneError::runtime(format!("fs_read: {error}")))?;
    if meta.len() > crate::MAX_BUFFER_BYTES as u64 {
        return Err(TinyOneError::runtime(format!(
            "fs_read: file size {} exceeds limit {}",
            meta.len(),
            crate::MAX_BUFFER_BYTES
        )));
    }
    let mut file = File::open(&path).map_err(|error| TinyOneError::runtime(format!("fs_read: {error}")))?;
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
    Ok(Value::Heap(context.heap().alloc_buffer_with(bytes)?))
}

pub fn b_fs_write(context: &mut TinyRuntimeContext, target: &Value, buffer: &Value) -> Result<Value> {
    let path = expect_string(context, target, "fs_write")?;
    let bytes = {
        let heap = context.heap();
        let object = heap.get(buffer)?;
        let HeapData::Buffer(bytes) = &object.data else {
            return Err(TinyOneError::runtime("fs_write expects a buffer payload"));
        };
        bytes.as_slice().to_vec()
    };
    std::fs::write(&path, &bytes).map_err(|error| TinyOneError::runtime(format!("fs_write: {error}")))?;
    Ok(Value::I64(bytes.len() as i64))
}

pub fn b_fs_exists(context: &TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "fs_exists")?;
    Ok(Value::I64(std::path::Path::new(&path).exists() as i64))
}

pub fn b_fs_list_dir(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let path = expect_string(context, target, "fs_list_dir")?;
    let mut sorted = BTreeMap::new();
    let mut name_bytes = 0usize;
    let entries = std::fs::read_dir(&path).map_err(|error| TinyOneError::runtime(format!("fs_list_dir: {error}")))?;
    for entry in entries {
        let entry = entry.map_err(|error| TinyOneError::runtime(format!("fs_list_dir: {error}")))?;
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
        names.push(Value::Heap(context.heap().alloc_string(name)?));
    }
    Ok(Value::Heap(context.heap().alloc_array(names)?))
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
    Ok(Value::I64(value))
}

pub fn b_math_abs(value: &Value) -> Result<Value> {
    let v = expect_int(value, "math_abs")?;
    let result = v
        .checked_abs()
        .ok_or_else(|| TinyOneError::runtime("Runtime.Memory_Overflow: math_abs"))?;
    Ok(Value::I64(result))
}

pub fn b_math_min(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "math_min")?;
    let b = expect_int(rhs, "math_min")?;
    Ok(Value::I64(a.min(b)))
}

pub fn b_math_max(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "math_max")?;
    let b = expect_int(rhs, "math_max")?;
    Ok(Value::I64(a.max(b)))
}

pub fn b_logic_and(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "logic_and")?;
    let b = expect_int(rhs, "logic_and")?;
    Ok(Value::I64(((a != 0) && (b != 0)) as i64))
}

pub fn b_logic_or(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "logic_or")?;
    let b = expect_int(rhs, "logic_or")?;
    Ok(Value::I64(((a != 0) || (b != 0)) as i64))
}

pub fn b_logic_not(value: &Value) -> Result<Value> {
    let v = expect_int(value, "logic_not")?;
    Ok(Value::I64((v == 0) as i64))
}

pub fn b_logic_xor(lhs: &Value, rhs: &Value) -> Result<Value> {
    let a = expect_int(lhs, "logic_xor")?;
    let b = expect_int(rhs, "logic_xor")?;
    Ok(Value::I64(((a != 0) ^ (b != 0)) as i64))
}

// ---------------------------------------------------------------------------
// Typed integer ops: widths enforced per typing_system.md.
// ---------------------------------------------------------------------------

pub fn b_type_of(context: &mut TinyRuntimeContext, value: &Value) -> Result<Value> {
    let name = match value {
        Value::I8(_) => TypeKind::I8.name(),
        Value::I16(_) => TypeKind::I16.name(),
        Value::I32(_) => TypeKind::I32.name(),
        Value::I64(_) | Value::U8(_) | Value::U16(_) | Value::U32(_) => {
            runtime_integer_type_name(value).unwrap_or(TypeKind::I64.name())
        }
        Value::U64(_) => TypeKind::U64.name(),
        Value::Float { kind, .. } => kind.name(),
        Value::Bool(_) => TypeKind::Bool.name(),
        Value::Unit => TypeKind::Unit.name(),
        Value::Null => TypeKind::Null.name(),
        Value::Function(_) => TypeKind::Function.name(),
        Value::Reference(_) => TypeKind::Reference.name(),
        Value::Phantom => TypeKind::Phantom.name(),
        Value::Zst(_) => TypeKind::Zst.name(),
        Value::Unsafe => TypeKind::Unsafe.name(),
        Value::Pointer(p) if p.kind == crate::runtime::value::PointerKind::Null && p.address == 0 => {
            TypeKind::Null.name()
        }
        Value::Pointer(_) => TypeKind::Pointer.name(),
        Value::Heap(_) => {
            let heap = context.heap();
            let object = heap.get(value)?;
            match &object.data {
                HeapData::String(_) => TypeKind::String.name(),
                HeapData::Array(_) => TypeKind::Array.name(),
                HeapData::Buffer(_) => TypeKind::Buffer.name(),
                HeapData::Struct(_) => {
                    if object.type_name == "tinyone.result.Result" {
                        TypeKind::Result.name()
                    } else if object.type_name == "tinyone.option.Option" {
                        TypeKind::Option.name()
                    } else {
                        TypeKind::Struct.name()
                    }
                }
                HeapData::Map(_) => TypeKind::Map.name(),
                HeapData::Cell(_) => TypeKind::Cell.name(),
                HeapData::Mutex(_) => TypeKind::Mutex.name(),
                HeapData::Atomic(_) => TypeKind::Atomic.name(),
                HeapData::Thread(_) => TypeKind::Thread.name(),
                HeapData::Enum { .. } => TypeKind::Enum.name(),
                HeapData::Char(_) => TypeKind::Char.name(),
                HeapData::CharBuffer(_) => TypeKind::CharBuffer.name(),
                HeapData::Vec(_) => TypeKind::Vec.name(),
                HeapData::Record(_) => TypeKind::Record.name(),
                HeapData::Dictionary(_) => TypeKind::Dictionary.name(),
                HeapData::Box(_) => TypeKind::Box.name(),
                HeapData::Alloc { .. } => TypeKind::Alloc.name(),
                HeapData::Closure { .. } => TypeKind::Closure.name(),
                HeapData::Sum { .. } => TypeKind::Sum.name(),
                HeapData::TaggedUnion { .. } => TypeKind::TaggedUnion.name(),
                HeapData::Result { .. } => TypeKind::Result.name(),
                HeapData::Option { .. } => TypeKind::Option.name(),
                HeapData::Dyn { .. } => TypeKind::Dyn.name(),
                HeapData::FileDescriptor(_) => TypeKind::FileDescriptor.name(),
            }
        }
    };
    // Drop heap guard before allocating.
    Ok(Value::Heap(context.heap().alloc_string(name.to_string())?))
}

pub fn b_type_id(context: &mut TinyRuntimeContext, type_name: &Value) -> Result<Value> {
    let name = expect_string(context, type_name, "type_id")?;
    let kind = parse_type_name(&name, "type_id")?;
    Ok(Value::I64(kind.type_id() as i64))
}

pub fn b_smallest_fit(value: &Value, context: &mut TinyRuntimeContext) -> Result<Value> {
    let v = expect_int(value, "smallest_fit")?;
    let kind = smallest_fit_literal(v);
    Ok(Value::Heap(context.heap().alloc_string(kind.name().to_string())?))
}

pub fn b_promote(context: &mut TinyRuntimeContext, lhs: &Value, rhs: &Value) -> Result<Value> {
    let lhs_name = expect_string(context, lhs, "promote")?;
    let rhs_name = expect_string(context, rhs, "promote")?;
    let lhs_kind = parse_type_name(&lhs_name, "promote")?;
    let rhs_kind = parse_type_name(&rhs_name, "promote")?;
    let kind = promote_integer(lhs_kind, rhs_kind)?;
    Ok(Value::Heap(context.heap().alloc_string(kind.name().to_string())?))
}

pub fn b_check_int_range(context: &TinyRuntimeContext, value: &Value, type_name: &Value) -> Result<Value> {
    let v = expect_int(value, "check_int_range")?;
    let name = expect_string(context, type_name, "check_int_range")?;
    let kind = parse_type_name(&name, "check_int_range")?;
    let _ =
        integer_range(kind).ok_or_else(|| TinyOneError::runtime(format!("{} is not an integer type", kind.name())))?;
    runtime_cast_int(&Value::I64(v), kind, "check_int_range")
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
    let result = op(lhs as i128, rhs as i128)
        .ok_or_else(|| TinyOneError::runtime(format!("Runtime.Memory_Overflow: {op_name} intermediate overflow")))?;
    integer_value_from_kind(kind, result, op_name)
}

pub fn b_typed_add(context: &TinyRuntimeContext, lhs: &Value, rhs: &Value, type_name: &Value) -> Result<Value> {
    typed_binary(context, lhs, rhs, type_name, "typed_add", i128::checked_add)
}

pub fn b_typed_sub(context: &TinyRuntimeContext, lhs: &Value, rhs: &Value, type_name: &Value) -> Result<Value> {
    typed_binary(context, lhs, rhs, type_name, "typed_sub", i128::checked_sub)
}

pub fn b_typed_mul(context: &TinyRuntimeContext, lhs: &Value, rhs: &Value, type_name: &Value) -> Result<Value> {
    typed_binary(context, lhs, rhs, type_name, "typed_mul", i128::checked_mul)
}

pub fn b_typed_div(context: &TinyRuntimeContext, lhs: &Value, rhs: &Value, type_name: &Value) -> Result<Value> {
    let lhs = expect_int(lhs, "typed_div")?;
    let rhs = expect_int(rhs, "typed_div")?;
    let name = expect_string(context, type_name, "typed_div")?;
    let kind = parse_type_name(&name, "typed_div")?;
    if rhs == 0 {
        return Err(TinyOneError::runtime("Runtime.Division_By_Zero"));
    }
    let quotient = (lhs as i128) / (rhs as i128);
    integer_value_from_kind(kind, quotient, "typed_div")
}

pub fn b_typed_neg(context: &TinyRuntimeContext, value: &Value, type_name: &Value) -> Result<Value> {
    let v = expect_int(value, "typed_neg")?;
    let name = expect_string(context, type_name, "typed_neg")?;
    let kind = parse_type_name(&name, "typed_neg")?;
    if !kind.is_signed() {
        return Err(TinyOneError::runtime(format!("typed_neg: {} is not signed", kind.name())));
    }
    let negated = (v as i128)
        .checked_neg()
        .ok_or_else(|| TinyOneError::runtime("Runtime.Memory_Overflow: typed_neg intermediate overflow"))?;
    integer_value_from_kind(kind, negated, "typed_neg")
}

pub fn b_assert(value: &Value, message: Option<&Value>, context: &TinyRuntimeContext) -> Result<Value> {
    let v = expect_int(value, "assert")?;
    if v == 0 {
        let detail = if let Some(message) = message {
            expect_string(context, message, "assert")?
        } else {
            "assertion failed".to_string()
        };
        return Err(TinyOneError::runtime(format!("Assertion failed: {detail}")));
    }
    Ok(Value::I64(1))
}
