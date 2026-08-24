//! Fixed-width byte encoding for [`Value`], used to store `Value`s inside
//! `RallocBytes`-backed containers (arrays, structs, maps, ...) as flat,
//! Ralloc-owned memory instead of a Rust-native `Vec<Value>`.
//!
//! Every `Value` encodes into exactly [`ENCODED_VALUE_BYTES`] bytes,
//! regardless of variant. This is no more wasteful than today's
//! `Vec<Value>`, which already pays `size_of::<Value>()` per slot
//! regardless of what's stored — `RawPointer`'s payload already dominates
//! that size, so per-element heap-byte-budget accounting is unaffected.
//!
//! All reads/writes are manual little-endian byte packing (`to_le_bytes`/
//! `from_le_bytes`) at fixed offsets — no `unsafe`, no reliance on Rust's
//! (unstable, layout-unspecified) enum representation. This mirrors how
//! `runtime::pointers::runtime_read_uint`/`runtime_write_uint` already pack
//! multi-byte integers by hand elsewhere in this codebase.

use crate::runtime::value::{CastKind, PointerKind};
use crate::{HeapRef, Op, RawPointer, Result, TinyOneError, TypeKind, Value, floor_div};

/// Total encoded width of one `Value`, in bytes.
pub(crate) const ENCODED_VALUE_BYTES: usize = 64;

/// Inline capacity for a field-pointer's field name when a `Value::Pointer`/
/// `Value::Reference` is stored inside a container. Normal stack-resident
/// pointers (the overwhelmingly common case) are unaffected by this cap —
/// `RawPointer.field` stays a plain, unbounded `String` there; this only
/// applies at the point a pointer *value* is written into a fixed-width
/// container slot.
const MAX_INLINE_FIELD_BYTES: usize = 27;

const TAG_I8: u8 = 0;
const TAG_I16: u8 = 1;
const TAG_I32: u8 = 2;
const TAG_I64: u8 = 3;
const TAG_U8: u8 = 4;
const TAG_U16: u8 = 5;
const TAG_U32: u8 = 6;
const TAG_U64: u8 = 7;
const TAG_FLOAT: u8 = 8;
const TAG_BOOL: u8 = 9;
const TAG_UNIT: u8 = 10;
const TAG_NULL: u8 = 11;
const TAG_FUNCTION: u8 = 12;
const TAG_REFERENCE: u8 = 13;
const TAG_PHANTOM: u8 = 14;
const TAG_ZST: u8 = 15;
const TAG_UNSAFE: u8 = 16;
const TAG_HEAP: u8 = 17;
const TAG_POINTER: u8 = 18;

// Byte layout within the `ENCODED_VALUE_BYTES`-wide slot. The "scalar"
// region and the "pointer-shaped" region are never both in use for the same
// `Value`, so they don't need to avoid overlapping in spirit — they're kept
// numerically disjoint anyway purely for clarity when reading this file.
const OFF_SCALAR: usize = 1; // 8 bytes: 1..9
const OFF_TYPE_KIND: usize = 9; // 1 byte: Float/Zst's TypeKind id
const OFF_ADDRESS: usize = 10; // 8 bytes: 10..18
const OFF_INDEX: usize = 18; // 8 bytes: 18..26
const OFF_GENERATION: usize = 26; // 8 bytes: 26..34
const OFF_POINTER_KIND: usize = 34; // 1 byte
const OFF_CAST_KIND: usize = 35; // 1 byte
const OFF_FIELD_LEN: usize = 36; // 1 byte
const OFF_FIELD_BYTES: usize = 37; // MAX_INLINE_FIELD_BYTES bytes: 37..64

const _: () = assert!(OFF_FIELD_BYTES + MAX_INLINE_FIELD_BYTES == ENCODED_VALUE_BYTES);

fn write_u64(out: &mut [u8; ENCODED_VALUE_BYTES], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u64(bytes: &[u8; ENCODED_VALUE_BYTES], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn write_i64(out: &mut [u8; ENCODED_VALUE_BYTES], offset: usize, value: i64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_i64(bytes: &[u8; ENCODED_VALUE_BYTES], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

pub(crate) fn encode_i64(value: i64) -> [u8; ENCODED_VALUE_BYTES] {
    crate::runtime::instrumentation::record_value_encode();
    encode_i64_raw(value)
}

fn encode_i64_raw(value: i64) -> [u8; ENCODED_VALUE_BYTES] {
    let mut out = [0u8; ENCODED_VALUE_BYTES];
    out[0] = TAG_I64;
    write_i64(&mut out, OFF_SCALAR, value);
    out
}

pub(crate) fn add_i64_in_place(bytes: &mut [u8; ENCODED_VALUE_BYTES], value: i64) -> Result<bool> {
    if bytes[0] != TAG_I64 {
        return Ok(false);
    }
    let next = checked_i64_result(
        i128::from(read_i64(bytes, OFF_SCALAR)) + i128::from(value),
        "Addition",
    )?;
    write_i64(bytes, OFF_SCALAR, next);
    Ok(true)
}

pub(crate) fn sub_i64_in_place(bytes: &mut [u8; ENCODED_VALUE_BYTES], value: i64) -> Result<bool> {
    if bytes[0] != TAG_I64 {
        return Ok(false);
    }
    let next = checked_i64_result(
        i128::from(read_i64(bytes, OFF_SCALAR)) - i128::from(value),
        "Subtraction",
    )?;
    write_i64(bytes, OFF_SCALAR, next);
    Ok(true)
}

pub(crate) fn compare_i64(
    bytes: &[u8; ENCODED_VALUE_BYTES],
    value: i64,
    op: Op,
) -> Result<Option<bool>> {
    if bytes[0] != TAG_I64 {
        return Ok(None);
    }
    let stored = read_i64(bytes, OFF_SCALAR);
    let result = match op {
        Op::Lt => stored < value,
        Op::Lte => stored <= value,
        Op::Gt => stored > value,
        Op::Gte => stored >= value,
        Op::Eq => stored == value,
        Op::Ne => stored != value,
        _ => {
            return Err(TinyOneError::runtime(format!(
                "Unsupported comparison opcode {op:?}"
            )));
        }
    };
    Ok(Some(result))
}

pub(crate) fn is_i64_zero(bytes: &[u8; ENCODED_VALUE_BYTES]) -> Option<bool> {
    (bytes[0] == TAG_I64).then(|| read_i64(bytes, OFF_SCALAR) == 0)
}

pub(crate) fn mul_i64(bytes: &[u8; ENCODED_VALUE_BYTES], value: i64) -> Result<Option<i64>> {
    if bytes[0] != TAG_I64 {
        return Ok(None);
    }
    checked_i64_product(read_i64(bytes, OFF_SCALAR), value).map(Some)
}

pub(crate) fn div_i64(bytes: &[u8; ENCODED_VALUE_BYTES], value: i64) -> Result<Option<i64>> {
    if bytes[0] != TAG_I64 {
        return Ok(None);
    }
    if value == 0 {
        return Err(TinyOneError::runtime("Division by zero"));
    }
    floor_div(read_i64(bytes, OFF_SCALAR), value)
        .map(Some)
        .ok_or_else(|| TinyOneError::runtime("Division overflow"))
}

pub(crate) fn mul_i64_in_place(bytes: &mut [u8; ENCODED_VALUE_BYTES], value: i64) -> Result<bool> {
    if bytes[0] != TAG_I64 {
        return Ok(false);
    }
    let next = checked_i64_product(read_i64(bytes, OFF_SCALAR), value)?;
    write_i64(bytes, OFF_SCALAR, next);
    Ok(true)
}

fn checked_i64_product(lhs: i64, rhs: i64) -> Result<i64> {
    checked_i64_result(i128::from(lhs) * i128::from(rhs), "Multiplication")
}

fn checked_i64_result(value: i128, operation: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        TinyOneError::runtime(format!(
            "Runtime.Memory_Overflow: {value} out of range for i64 in {operation}"
        ))
    })
}

pub(crate) fn div_i64_in_place(bytes: &mut [u8; ENCODED_VALUE_BYTES], value: i64) -> Result<bool> {
    if bytes[0] != TAG_I64 {
        return Ok(false);
    }
    if value == 0 {
        return Err(TinyOneError::runtime("Division by zero"));
    }
    let next = floor_div(read_i64(bytes, OFF_SCALAR), value)
        .ok_or_else(|| TinyOneError::runtime("Division overflow"))?;
    write_i64(bytes, OFF_SCALAR, next);
    Ok(true)
}

/// Reads only the heap identity needed to validate an encoded pointer key.
/// This deliberately avoids allocating/decoding its field-name payload.
pub(crate) fn encoded_pointer_base(
    bytes: &[u8; ENCODED_VALUE_BYTES],
) -> Option<(usize, u64, PointerKind)> {
    if bytes[0] != TAG_POINTER {
        return None;
    }
    let kind = PointerKind::from_u8(bytes[OFF_POINTER_KIND])?;
    Some((
        read_u64(bytes, OFF_ADDRESS) as usize,
        read_u64(bytes, OFF_GENERATION),
        kind,
    ))
}

fn encode_pointer(out: &mut [u8; ENCODED_VALUE_BYTES], pointer: &RawPointer) -> Result<()> {
    write_u64(out, OFF_ADDRESS, pointer.address as u64);
    write_i64(out, OFF_INDEX, pointer.index);
    write_u64(out, OFF_GENERATION, pointer.generation);
    out[OFF_POINTER_KIND] = pointer.kind.to_u8();
    out[OFF_CAST_KIND] = pointer.cast.to_u8();

    let field_bytes = pointer.field.as_bytes();
    if field_bytes.len() > MAX_INLINE_FIELD_BYTES {
        return Err(TinyOneError::runtime(format!(
            "field name {:?} is too long ({} bytes) to store inside a container; max {MAX_INLINE_FIELD_BYTES} bytes",
            pointer.field,
            field_bytes.len()
        )));
    }
    out[OFF_FIELD_LEN] = field_bytes.len() as u8;
    out[OFF_FIELD_BYTES..OFF_FIELD_BYTES + field_bytes.len()].copy_from_slice(field_bytes);
    Ok(())
}

fn decode_pointer(bytes: &[u8; ENCODED_VALUE_BYTES]) -> RawPointer {
    let address = read_u64(bytes, OFF_ADDRESS) as usize;
    let index = read_i64(bytes, OFF_INDEX);
    let generation = read_u64(bytes, OFF_GENERATION);
    let kind = PointerKind::from_u8(bytes[OFF_POINTER_KIND])
        .expect("decode_pointer: invalid PointerKind byte in encoded Value");
    let cast = CastKind::from_u8(bytes[OFF_CAST_KIND])
        .expect("decode_pointer: invalid CastKind byte in encoded Value");
    let field_len = bytes[OFF_FIELD_LEN] as usize;
    let field_bytes = &bytes[OFF_FIELD_BYTES..OFF_FIELD_BYTES + field_len];
    let field = std::str::from_utf8(field_bytes)
        .expect("decode_pointer: invalid UTF-8 in encoded field name")
        .to_owned();
    RawPointer::new(address, kind, index, field, generation, cast)
}

/// Encodes `value` into a fixed-width byte slot.
///
/// Fails only if `value` is a `Pointer`/`Reference` whose field name exceeds
/// [`MAX_INLINE_FIELD_BYTES`] — every other `Value` variant always succeeds.
pub(crate) fn encode_value(value: &Value) -> Result<[u8; ENCODED_VALUE_BYTES]> {
    crate::runtime::instrumentation::record_value_encode();
    let mut out = [0u8; ENCODED_VALUE_BYTES];
    match value {
        Value::I8(v) => {
            out[0] = TAG_I8;
            out[OFF_SCALAR] = *v as u8;
        }
        Value::I16(v) => {
            out[0] = TAG_I16;
            out[OFF_SCALAR..OFF_SCALAR + 2].copy_from_slice(&v.to_le_bytes());
        }
        Value::I32(v) => {
            out[0] = TAG_I32;
            out[OFF_SCALAR..OFF_SCALAR + 4].copy_from_slice(&v.to_le_bytes());
        }
        Value::I64(v) => {
            return Ok(encode_i64_raw(*v));
        }
        Value::U8(v) => {
            out[0] = TAG_U8;
            out[OFF_SCALAR] = *v;
        }
        Value::U16(v) => {
            out[0] = TAG_U16;
            out[OFF_SCALAR..OFF_SCALAR + 2].copy_from_slice(&v.to_le_bytes());
        }
        Value::U32(v) => {
            out[0] = TAG_U32;
            out[OFF_SCALAR..OFF_SCALAR + 4].copy_from_slice(&v.to_le_bytes());
        }
        Value::U64(v) => {
            out[0] = TAG_U64;
            out[OFF_SCALAR..OFF_SCALAR + 8].copy_from_slice(&v.to_le_bytes());
        }
        Value::Float { kind, bits } => {
            out[0] = TAG_FLOAT;
            out[OFF_SCALAR..OFF_SCALAR + 8].copy_from_slice(&bits.to_le_bytes());
            out[OFF_TYPE_KIND] = kind.type_id() as u8;
        }
        Value::Bool(b) => {
            out[0] = TAG_BOOL;
            out[OFF_SCALAR] = *b as u8;
        }
        Value::Unit => out[0] = TAG_UNIT,
        Value::Null => out[0] = TAG_NULL,
        Value::Function(id) => {
            out[0] = TAG_FUNCTION;
            out[OFF_SCALAR..OFF_SCALAR + 4].copy_from_slice(&id.to_le_bytes());
        }
        Value::Reference(pointer) => {
            out[0] = TAG_REFERENCE;
            encode_pointer(&mut out, pointer)?;
        }
        Value::Phantom => out[0] = TAG_PHANTOM,
        Value::Zst(kind) => {
            out[0] = TAG_ZST;
            out[OFF_TYPE_KIND] = kind.type_id() as u8;
        }
        Value::Unsafe => out[0] = TAG_UNSAFE,
        Value::Heap(HeapRef {
            address,
            generation,
        }) => {
            out[0] = TAG_HEAP;
            write_u64(&mut out, OFF_ADDRESS, *address as u64);
            write_u64(&mut out, OFF_GENERATION, *generation);
        }
        Value::Pointer(pointer) => {
            out[0] = TAG_POINTER;
            encode_pointer(&mut out, pointer)?;
        }
    }
    Ok(out)
}

/// Decodes a `Value` previously produced by [`encode_value`].
///
/// Panics if `bytes` wasn't produced by `encode_value` (corrupt tag, or an
/// out-of-range `PointerKind`/`CastKind`/UTF-8 field byte) — this is always
/// a bug in the caller, not a condition a well-formed container can reach.
pub(crate) fn decode_value(bytes: &[u8; ENCODED_VALUE_BYTES]) -> Value {
    crate::runtime::instrumentation::record_value_decode();
    match bytes[0] {
        TAG_I8 => Value::I8(bytes[OFF_SCALAR] as i8),
        TAG_I16 => Value::I16(i16::from_le_bytes(
            bytes[OFF_SCALAR..OFF_SCALAR + 2].try_into().unwrap(),
        )),
        TAG_I32 => Value::I32(i32::from_le_bytes(
            bytes[OFF_SCALAR..OFF_SCALAR + 4].try_into().unwrap(),
        )),
        TAG_I64 => Value::I64(read_i64(bytes, OFF_SCALAR)),
        TAG_U8 => Value::U8(bytes[OFF_SCALAR]),
        TAG_U16 => Value::U16(u16::from_le_bytes(
            bytes[OFF_SCALAR..OFF_SCALAR + 2].try_into().unwrap(),
        )),
        TAG_U32 => Value::U32(u32::from_le_bytes(
            bytes[OFF_SCALAR..OFF_SCALAR + 4].try_into().unwrap(),
        )),
        TAG_U64 => Value::U64(read_u64(bytes, OFF_SCALAR)),
        TAG_FLOAT => {
            let bits = f64::from_le_bytes(bytes[OFF_SCALAR..OFF_SCALAR + 8].try_into().unwrap());
            let kind = TypeKind::from_type_id(bytes[OFF_TYPE_KIND] as u16)
                .expect("decode_value: invalid TypeKind byte for Float");
            Value::Float { kind, bits }
        }
        TAG_BOOL => Value::Bool(bytes[OFF_SCALAR] != 0),
        TAG_UNIT => Value::Unit,
        TAG_NULL => Value::Null,
        TAG_FUNCTION => Value::Function(u32::from_le_bytes(
            bytes[OFF_SCALAR..OFF_SCALAR + 4].try_into().unwrap(),
        )),
        TAG_REFERENCE => Value::Reference(decode_pointer(bytes)),
        TAG_PHANTOM => Value::Phantom,
        TAG_ZST => {
            let kind = TypeKind::from_type_id(bytes[OFF_TYPE_KIND] as u16)
                .expect("decode_value: invalid TypeKind byte for Zst");
            Value::Zst(kind)
        }
        TAG_UNSAFE => Value::Unsafe,
        TAG_HEAP => Value::Heap(HeapRef {
            address: read_u64(bytes, OFF_ADDRESS) as usize,
            generation: read_u64(bytes, OFF_GENERATION),
        }),
        TAG_POINTER => Value::Pointer(decode_pointer(bytes)),
        other => unreachable!("decode_value: invalid Value tag byte {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: Value) {
        let encoded = encode_value(&value).unwrap_or_else(|e| {
            panic!("encode_value failed for {value:?}: {e}");
        });
        let decoded = decode_value(&encoded);
        assert_eq!(
            decoded, value,
            "round trip mismatch for {value:?} (decoded: {decoded:?})"
        );
    }

    #[test]
    fn specialized_i64_updates_preserve_encoding_and_overflow_checks() {
        let mut encoded = encode_i64(40);
        assert!(add_i64_in_place(&mut encoded, 2).unwrap());
        assert_eq!(decode_value(&encoded), Value::I64(42));
        assert!(sub_i64_in_place(&mut encoded, 4).unwrap());
        assert_eq!(decode_value(&encoded), Value::I64(38));

        let mut maximum = encode_i64(i64::MAX);
        assert!(add_i64_in_place(&mut maximum, 1).is_err());
        assert_eq!(decode_value(&maximum), Value::I64(i64::MAX));
    }

    #[test]
    fn specialized_i64_update_declines_other_value_kinds() {
        let mut encoded = encode_value(&Value::I8(7)).unwrap();
        assert!(!add_i64_in_place(&mut encoded, 1).unwrap());
        assert_eq!(decode_value(&encoded), Value::I8(7));
    }

    #[test]
    fn specialized_i64_comparison_handles_all_relations_and_declines_other_kinds() {
        let encoded = encode_i64(3);
        for (op, expected) in [
            (Op::Lt, true),
            (Op::Lte, true),
            (Op::Gt, false),
            (Op::Gte, false),
            (Op::Eq, false),
            (Op::Ne, true),
        ] {
            assert_eq!(compare_i64(&encoded, 4, op).unwrap(), Some(expected));
        }

        for value in [
            Value::I8(3),
            Value::U64(3),
            Value::Float {
                kind: TypeKind::Fp64,
                bits: 3.0,
            },
        ] {
            let encoded = encode_value(&value).unwrap();
            assert_eq!(compare_i64(&encoded, 4, Op::Lt).unwrap(), None);
        }
    }

    #[test]
    fn specialized_i64_zero_test_declines_other_value_kinds() {
        assert_eq!(is_i64_zero(&encode_i64(0)), Some(true));
        assert_eq!(is_i64_zero(&encode_i64(-1)), Some(false));
        let encoded = encode_value(&Value::U8(0)).unwrap();
        assert_eq!(is_i64_zero(&encoded), None);
    }

    #[test]
    fn specialized_i64_mul_and_floor_div_preserve_errors_and_decline_other_kinds() {
        let encoded = encode_i64(-7);
        assert_eq!(mul_i64(&encoded, 6).unwrap(), Some(-42));
        assert_eq!(div_i64(&encoded, 3).unwrap(), Some(-3));
        assert!(div_i64(&encoded, 0).is_err());

        let maximum = encode_i64(i64::MAX);
        assert!(mul_i64(&maximum, 2).is_err());
        let minimum = encode_i64(i64::MIN);
        assert!(div_i64(&minimum, -1).is_err());

        for value in [
            Value::I8(3),
            Value::U64(3),
            Value::Float {
                kind: TypeKind::Fp64,
                bits: 3.0,
            },
        ] {
            let encoded = encode_value(&value).unwrap();
            assert_eq!(mul_i64(&encoded, 2).unwrap(), None);
            assert_eq!(div_i64(&encoded, 2).unwrap(), None);
        }
    }

    #[test]
    fn specialized_i64_mul_and_floor_div_updates_are_in_place_and_checked() {
        let mut encoded = encode_i64(-7);
        assert!(mul_i64_in_place(&mut encoded, 6).unwrap());
        assert_eq!(decode_value(&encoded), Value::I64(-42));
        assert!(div_i64_in_place(&mut encoded, 5).unwrap());
        assert_eq!(decode_value(&encoded), Value::I64(-9));

        let mut maximum = encode_i64(i64::MAX);
        assert!(mul_i64_in_place(&mut maximum, 2).is_err());
        assert_eq!(decode_value(&maximum), Value::I64(i64::MAX));
        let mut minimum = encode_i64(i64::MIN);
        assert!(div_i64_in_place(&mut minimum, -1).is_err());
        assert_eq!(decode_value(&minimum), Value::I64(i64::MIN));
    }

    #[test]
    fn round_trips_every_scalar_variant() {
        round_trip(Value::I8(-42));
        round_trip(Value::I16(-1234));
        round_trip(Value::I32(-123_456));
        round_trip(Value::I64(-1_234_567_890_123));
        round_trip(Value::U8(200));
        round_trip(Value::U16(50_000));
        round_trip(Value::U32(3_000_000_000));
        round_trip(Value::U64(u64::MAX));
        round_trip(Value::Bool(true));
        round_trip(Value::Bool(false));
        round_trip(Value::Unit);
        round_trip(Value::Null);
        round_trip(Value::Function(7));
        round_trip(Value::Phantom);
        round_trip(Value::Unsafe);
    }

    #[test]
    fn round_trips_float_and_zst_preserving_type_kind() {
        round_trip(Value::Float {
            kind: TypeKind::Fp32,
            bits: 1.5,
        });
        round_trip(Value::Float {
            kind: TypeKind::Fp64,
            bits: f64::NAN.to_bits() as f64, // exercise a non-trivial bit pattern path
        });
        round_trip(Value::Zst(TypeKind::Phantom));
        round_trip(Value::Zst(TypeKind::Unsafe));
    }

    #[test]
    fn round_trips_heap_ref() {
        round_trip(Value::Heap(HeapRef {
            address: 12345,
            generation: 9,
        }));
    }

    #[test]
    fn round_trips_pointer_and_reference_variants() {
        round_trip(Value::Pointer(RawPointer::new(
            10,
            PointerKind::Array,
            3,
            "",
            2,
            CastKind::I32,
        )));
        round_trip(Value::Pointer(RawPointer::new(
            10,
            PointerKind::Field,
            0,
            "short_name",
            2,
            CastKind::None,
        )));
        round_trip(Value::Reference(RawPointer::new(
            1,
            PointerKind::Null,
            0,
            "",
            0,
            CastKind::None,
        )));
    }

    #[test]
    fn pointer_field_name_at_exact_cap_round_trips() {
        let name = "x".repeat(MAX_INLINE_FIELD_BYTES);
        round_trip(Value::Pointer(RawPointer::new(
            1,
            PointerKind::Field,
            0,
            name,
            1,
            CastKind::None,
        )));
    }

    #[test]
    fn pointer_field_name_over_cap_is_a_clean_error_not_a_panic() {
        let name = "x".repeat(MAX_INLINE_FIELD_BYTES + 1);
        let value = Value::Pointer(RawPointer::new(
            1,
            PointerKind::Field,
            0,
            name,
            1,
            CastKind::None,
        ));
        let err = encode_value(&value).unwrap_err();
        assert!(format!("{err}").contains("too long"));
    }
}
