use crate::{Op, Result, TinyOneError, TypeKind, Value, round_to_kind};

pub(crate) fn expect_int(value: &Value, operation: &str) -> Result<i64> {
    let raw = runtime_integer_value(value, operation)?;
    i64::try_from(raw).map_err(|_| {
        TinyOneError::runtime(format!("{operation} integer value is too large for i64"))
    })
}

pub(crate) fn runtime_integer_kind(value: &Value) -> Option<TypeKind> {
    match value {
        Value::I8(_) => Some(TypeKind::I8),
        Value::I16(_) => Some(TypeKind::I16),
        Value::I32(_) => Some(TypeKind::I32),
        Value::I64(_) => Some(TypeKind::I64),
        Value::U8(_) => Some(TypeKind::U8),
        Value::U16(_) => Some(TypeKind::U16),
        Value::U32(_) => Some(TypeKind::U32),
        Value::U64(_) => Some(TypeKind::U64),
        _ => None,
    }
}

pub(crate) fn runtime_integer_value(value: &Value, operation: &str) -> Result<i128> {
    match value {
        Value::I8(v) => Ok(*v as i128),
        Value::I16(v) => Ok(*v as i128),
        Value::I32(v) => Ok(*v as i128),
        Value::I64(v) => Ok(*v as i128),
        Value::U8(v) => Ok(*v as i128),
        Value::U16(v) => Ok(*v as i128),
        Value::U32(v) => Ok(*v as i128),
        Value::U64(v) => Ok(*v as i128),
        _ => Err(TinyOneError::runtime(format!(
            "{operation} expects integer operands"
        ))),
    }
}

pub(crate) fn runtime_cast_int(value: &Value, kind: TypeKind, operation: &str) -> Result<Value> {
    let value = runtime_integer_value(value, operation)?;
    integer_value_from_kind(kind, value, operation)
}

pub(crate) fn integer_value_from_kind(
    kind: TypeKind,
    value: i128,
    operation: &str,
) -> Result<Value> {
    use crate::runtime::typing::check_integer_range;
    let checked = check_integer_range(kind, value).map_err(|_| {
        TinyOneError::runtime(format!(
            "Runtime.Memory_Overflow: {value} out of range for {} in {operation}",
            kind.name()
        ))
    })?;
    Ok(match kind {
        TypeKind::I8 => Value::I8(checked as i8),
        TypeKind::I16 => Value::I16(checked as i16),
        TypeKind::I32 => Value::I32(checked as i32),
        TypeKind::I64 => Value::I64(checked as i64),
        TypeKind::U8 => Value::U8(checked as u8),
        TypeKind::U16 => Value::U16(checked as u16),
        TypeKind::U32 => Value::U32(checked as u32),
        TypeKind::U64 => Value::U64(checked as u64),
        _ => {
            return Err(TinyOneError::runtime(format!(
                "{operation}: {} is not supported as a runtime integer value",
                kind.name()
            )));
        }
    })
}

fn unsigned_rank(kind: TypeKind) -> Option<u8> {
    Some(match kind {
        TypeKind::U8 => 1,
        TypeKind::U16 => 2,
        TypeKind::U32 => 3,
        TypeKind::U64 => 4,
        _ => return None,
    })
}

fn unsigned_from_rank(rank: u8) -> TypeKind {
    match rank {
        1 => TypeKind::U8,
        2 => TypeKind::U16,
        3 => TypeKind::U32,
        _ => TypeKind::U64,
    }
}

fn unsigned_max(kind: TypeKind) -> i128 {
    match kind {
        TypeKind::U8 => u8::MAX as i128,
        TypeKind::U16 => u16::MAX as i128,
        TypeKind::U32 => u32::MAX as i128,
        TypeKind::U64 => u64::MAX as i128,
        _ => 0,
    }
}

fn arithmetic_kind(lhs: &Value, rhs: &Value, operation: &str) -> Result<TypeKind> {
    let lhs_kind = runtime_integer_kind(lhs)
        .ok_or_else(|| TinyOneError::runtime(format!("{operation} expects integer operands")))?;
    let rhs_kind = runtime_integer_kind(rhs)
        .ok_or_else(|| TinyOneError::runtime(format!("{operation} expects integer operands")))?;
    if lhs_kind == rhs_kind {
        return Ok(lhs_kind);
    }
    match (unsigned_rank(lhs_kind), unsigned_rank(rhs_kind)) {
        (Some(lhs_rank), Some(rhs_rank)) => {
            return Ok(unsigned_from_rank(lhs_rank.max(rhs_rank)));
        }
        (Some(_), None) if rhs_kind == TypeKind::I64 => {
            let rhs_value = runtime_integer_value(rhs, operation)?;
            if rhs_value >= 0 && rhs_value <= unsigned_max(lhs_kind) {
                return Ok(lhs_kind);
            }
        }
        (None, Some(_)) if lhs_kind == TypeKind::I64 => {
            let lhs_value = runtime_integer_value(lhs, operation)?;
            if lhs_value >= 0 && lhs_value <= unsigned_max(rhs_kind) {
                return Ok(rhs_kind);
            }
        }
        _ => {}
    }
    Ok(TypeKind::I64)
}

// ── Float support ────────────────────────────────────────────────────────────
//
// A bare float literal is always `TypeKind::Fp64` (see `Op::PushFloat`);
// there is no float-typed literal suffix syntax. `fp8`/`fp16`/`fp32` values
// are only reachable via the matching cast builtin (`fp8(x)` etc., see
// `runtime::stdlib::b_float_cast`), which also rounds to that format's
// precision. Arithmetic results are rounded to the resolved result kind's
// precision too — see `round_to_kind` calls below.

fn runtime_float_kind(value: &Value) -> Option<TypeKind> {
    match value {
        Value::Float { kind, .. } => Some(*kind),
        _ => None,
    }
}

fn is_float_operand(value: &Value) -> bool {
    matches!(value, Value::Float { .. })
}

/// Rank used to pick the wider of two float kinds when both operands are
/// floats (e.g. a hypothetical `fp32 + fp64`). Defensive default of widest
/// for any kind outside the three float `TypeKind`s, since that should be
/// unreachable given `runtime_float_kind` only ever returns those three.
fn float_rank(kind: TypeKind) -> u8 {
    match kind {
        TypeKind::Fp8 => 0,
        TypeKind::Fp16 => 1,
        TypeKind::Fp32 => 2,
        TypeKind::Fp64 => 3,
        _ => 3,
    }
}

fn float_arithmetic_kind(lhs: &Value, rhs: &Value) -> TypeKind {
    match (runtime_float_kind(lhs), runtime_float_kind(rhs)) {
        (Some(a), Some(b)) => {
            if float_rank(a) >= float_rank(b) {
                a
            } else {
                b
            }
        }
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => TypeKind::Fp64,
    }
}

/// Converts an operand to `f64`, promoting any integer value. Callers only
/// reach this once at least one operand is already known to be a float (via
/// `is_float_operand`), so the non-float side here is always an int being
/// promoted to participate in float arithmetic.
fn runtime_numeric_as_f64(value: &Value, operation: &str) -> Result<f64> {
    match value {
        Value::Float { bits, .. } => Ok(*bits),
        _ => Ok(runtime_integer_value(value, operation)? as f64),
    }
}

pub(crate) fn runtime_add_int(lhs: Value, rhs: Value) -> Result<Value> {
    match (lhs, rhs) {
        (Value::I64(lhs), Value::I64(rhs)) => {
            checked_fast_i64_result(i128::from(lhs) + i128::from(rhs), "Addition")
        }
        (lhs, rhs) => runtime_add(lhs, rhs),
    }
}

pub(crate) fn runtime_add(lhs: Value, rhs: Value) -> Result<Value> {
    if is_float_operand(&lhs) || is_float_operand(&rhs) {
        let kind = float_arithmetic_kind(&lhs, &rhs);
        let lhs = runtime_numeric_as_f64(&lhs, "Addition")?;
        let rhs = runtime_numeric_as_f64(&rhs, "Addition")?;
        return Ok(Value::Float {
            kind,
            bits: round_to_kind(lhs + rhs, kind),
        });
    }
    let kind = arithmetic_kind(&lhs, &rhs, "Addition")?;
    let lhs = runtime_integer_value(&lhs, "Addition")?;
    let rhs = runtime_integer_value(&rhs, "Addition")?;
    let result = lhs
        .checked_add(rhs)
        .ok_or_else(|| TinyOneError::runtime("Addition overflow"))?;
    integer_value_from_kind(kind, result, "Addition")
}

pub(crate) fn runtime_sub_int(lhs: Value, rhs: Value) -> Result<Value> {
    match (lhs, rhs) {
        (Value::I64(lhs), Value::I64(rhs)) => {
            checked_fast_i64_result(i128::from(lhs) - i128::from(rhs), "Subtraction")
        }
        (lhs, rhs) => runtime_sub(lhs, rhs),
    }
}

pub(crate) fn runtime_sub(lhs: Value, rhs: Value) -> Result<Value> {
    if is_float_operand(&lhs) || is_float_operand(&rhs) {
        let kind = float_arithmetic_kind(&lhs, &rhs);
        let lhs = runtime_numeric_as_f64(&lhs, "Subtraction")?;
        let rhs = runtime_numeric_as_f64(&rhs, "Subtraction")?;
        return Ok(Value::Float {
            kind,
            bits: round_to_kind(lhs - rhs, kind),
        });
    }
    let kind = arithmetic_kind(&lhs, &rhs, "Subtraction")?;
    let lhs = runtime_integer_value(&lhs, "Subtraction")?;
    let rhs = runtime_integer_value(&rhs, "Subtraction")?;
    let result = lhs
        .checked_sub(rhs)
        .ok_or_else(|| TinyOneError::runtime("Subtraction overflow"))?;
    integer_value_from_kind(kind, result, "Subtraction")
}

pub(crate) fn runtime_mul_int(lhs: Value, rhs: Value) -> Result<Value> {
    match (lhs, rhs) {
        (Value::I64(lhs), Value::I64(rhs)) => {
            checked_fast_i64_result(i128::from(lhs) * i128::from(rhs), "Multiplication")
        }
        (lhs, rhs) => runtime_mul(lhs, rhs),
    }
}

fn checked_fast_i64_result(value: i128, operation: &str) -> Result<Value> {
    i64::try_from(value).map(Value::I64).map_err(|_| {
        TinyOneError::runtime(format!(
            "Runtime.Memory_Overflow: {value} out of range for i64 in {operation}"
        ))
    })
}

pub(crate) fn runtime_mul(lhs: Value, rhs: Value) -> Result<Value> {
    if is_float_operand(&lhs) || is_float_operand(&rhs) {
        let kind = float_arithmetic_kind(&lhs, &rhs);
        let lhs = runtime_numeric_as_f64(&lhs, "Multiplication")?;
        let rhs = runtime_numeric_as_f64(&rhs, "Multiplication")?;
        return Ok(Value::Float {
            kind,
            bits: round_to_kind(lhs * rhs, kind),
        });
    }
    let kind = arithmetic_kind(&lhs, &rhs, "Multiplication")?;
    let lhs = runtime_integer_value(&lhs, "Multiplication")?;
    let rhs = runtime_integer_value(&rhs, "Multiplication")?;
    let result = lhs
        .checked_mul(rhs)
        .ok_or_else(|| TinyOneError::runtime("Multiplication overflow"))?;
    integer_value_from_kind(kind, result, "Multiplication")
}

pub(crate) fn checked_div_int(lhs: Value, rhs: Value) -> Result<Value> {
    match (lhs, rhs) {
        (Value::I64(_), Value::I64(0)) => Err(TinyOneError::runtime("Division by zero")),
        (Value::I64(lhs), Value::I64(rhs)) => floor_div(lhs, rhs)
            .map(Value::I64)
            .ok_or_else(|| TinyOneError::runtime("Division overflow")),
        (lhs, rhs) => checked_div(lhs, rhs),
    }
}

pub(crate) fn checked_div(lhs: Value, rhs: Value) -> Result<Value> {
    if is_float_operand(&lhs) || is_float_operand(&rhs) {
        let kind = float_arithmetic_kind(&lhs, &rhs);
        let lhs_value = runtime_numeric_as_f64(&lhs, "Division")?;
        let rhs_value = runtime_numeric_as_f64(&rhs, "Division")?;
        if rhs_value == 0.0 {
            return Err(TinyOneError::runtime("Division by zero"));
        }
        return Ok(Value::Float {
            kind,
            bits: round_to_kind(lhs_value / rhs_value, kind),
        });
    }
    let kind = arithmetic_kind(&lhs, &rhs, "Division")?;
    let lhs_value = runtime_integer_value(&lhs, "Division")?;
    let rhs_value = runtime_integer_value(&rhs, "Division")?;
    if rhs_value == 0 {
        return Err(TinyOneError::runtime("Division by zero"));
    }
    let result = if kind == TypeKind::I64 {
        let lhs = i64::try_from(lhs_value)
            .map_err(|_| TinyOneError::runtime("Division left operand is too large"))?;
        let rhs = i64::try_from(rhs_value)
            .map_err(|_| TinyOneError::runtime("Division right operand is too large"))?;
        floor_div(lhs, rhs).ok_or_else(|| TinyOneError::runtime("Division overflow"))? as i128
    } else {
        lhs_value
            .checked_div(rhs_value)
            .ok_or_else(|| TinyOneError::runtime("Division overflow"))?
    };
    integer_value_from_kind(kind, result, "Division")
}

pub(crate) fn floor_div(lhs: i64, rhs: i64) -> Option<i64> {
    let quotient = lhs.checked_div(rhs)?;
    let remainder = lhs.checked_rem(rhs)?;
    if remainder != 0 && ((remainder > 0) != (rhs > 0)) {
        quotient.checked_sub(1)
    } else {
        Some(quotient)
    }
}

pub(crate) fn checked_non_negative_usize(value: i64, operation: &str) -> Result<usize> {
    if value < 0 {
        return Err(TinyOneError::runtime(format!(
            "{operation} must be non-negative"
        )));
    }
    usize::try_from(value).map_err(|_| TinyOneError::runtime(format!("{operation} is too large")))
}

pub(crate) fn checked_bounded_len(value: i64, operation: &str, max: usize) -> Result<usize> {
    let value = checked_non_negative_usize(value, operation)?;
    if value > max {
        return Err(TinyOneError::runtime(format!(
            "{operation} exceeds maximum length {max}"
        )));
    }
    Ok(value)
}

pub(crate) fn checked_collection_index(index: i64, len: usize, kind: &str) -> Result<usize> {
    if index < 0 {
        return Err(TinyOneError::runtime(format!(
            "{kind} index {index} out of bounds"
        )));
    }
    let index = usize::try_from(index)
        .map_err(|_| TinyOneError::runtime(format!("{kind} index {index} is too large")))?;
    if index >= len {
        return Err(TinyOneError::runtime(format!(
            "{kind} index {index} out of bounds"
        )));
    }
    Ok(index)
}

pub(crate) fn checked_byte_range(
    offset: i64,
    width: usize,
    len: usize,
    operation: &str,
) -> Result<usize> {
    if offset < 0 {
        return Err(TinyOneError::runtime(format!(
            "{operation} out of bounds at byte offset {offset}"
        )));
    }
    let offset = usize::try_from(offset).map_err(|_| {
        TinyOneError::runtime(format!("{operation} byte offset {offset} is too large"))
    })?;
    let end = offset.checked_add(width).ok_or_else(|| {
        TinyOneError::runtime(format!(
            "{operation} byte range overflows at offset {offset}"
        ))
    })?;
    if end > len {
        return Err(TinyOneError::runtime(format!(
            "{operation} out of bounds at byte offset {offset}"
        )));
    }
    Ok(offset)
}

pub(crate) fn checked_stack_count(stack_len: usize, count: usize) -> Result<()> {
    if count > stack_len {
        return Err(TinyOneError::runtime("Stack underflow"));
    }
    Ok(())
}

pub(crate) fn pop_args(stack: &mut Vec<Value>, count: usize) -> Result<Vec<Value>> {
    checked_stack_count(stack.len(), count)?;
    Ok(stack.split_off(stack.len() - count))
}

pub(crate) fn checked_payload_bytes(count: usize, unit: usize, operation: &str) -> Result<usize> {
    count
        .checked_mul(unit)
        .ok_or_else(|| TinyOneError::runtime(format!("{operation} payload is too large")))
}

pub(crate) fn runtime_neg(value: Value) -> Result<Value> {
    if let Value::Float { kind, bits } = value {
        return Ok(Value::Float { kind, bits: -bits });
    }
    let kind = runtime_integer_kind(&value)
        .ok_or_else(|| TinyOneError::runtime("Negation expects integer operands"))?;
    if kind.is_unsigned() {
        return Err(TinyOneError::runtime(
            "Negation expects a signed integer operand",
        ));
    }
    let value = runtime_integer_value(&value, "Negation")?;
    let result = value
        .checked_neg()
        .ok_or_else(|| TinyOneError::runtime("Negation overflow"))?;
    integer_value_from_kind(kind, result, "Negation")
}

pub(crate) fn runtime_compare_int(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
    match (lhs, rhs) {
        (Value::I64(lhs), Value::I64(rhs)) => {
            let result = match op {
                Op::Lt => lhs < rhs,
                Op::Lte => lhs <= rhs,
                Op::Gt => lhs > rhs,
                Op::Gte => lhs >= rhs,
                Op::Eq => lhs == rhs,
                Op::Ne => lhs != rhs,
                _ => {
                    return Err(TinyOneError::runtime(format!(
                        "Unsupported comparison opcode {op:?}"
                    )));
                }
            };
            Ok(Value::Bool(result))
        }
        (lhs, rhs) => runtime_compare(op, lhs, rhs),
    }
}

pub(crate) fn runtime_compare(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
    if is_float_operand(&lhs) || is_float_operand(&rhs) {
        let lhs = runtime_numeric_as_f64(&lhs, op.name())?;
        let rhs = runtime_numeric_as_f64(&rhs, op.name())?;
        let result = match op {
            Op::Lt => lhs < rhs,
            Op::Lte => lhs <= rhs,
            Op::Gt => lhs > rhs,
            Op::Gte => lhs >= rhs,
            Op::Eq => lhs == rhs,
            Op::Ne => lhs != rhs,
            _ => {
                return Err(TinyOneError::runtime(format!(
                    "Unsupported comparison opcode {op:?}"
                )));
            }
        };
        return Ok(Value::Bool(result));
    }
    let lhs = runtime_integer_value(&lhs, op.name())?;
    let rhs = runtime_integer_value(&rhs, op.name())?;
    let result = match op {
        Op::Lt => lhs < rhs,
        Op::Lte => lhs <= rhs,
        Op::Gt => lhs > rhs,
        Op::Gte => lhs >= rhs,
        Op::Eq => lhs == rhs,
        Op::Ne => lhs != rhs,
        _ => {
            return Err(TinyOneError::runtime(format!(
                "Unsupported comparison opcode {op:?}"
            )));
        }
    };
    Ok(Value::Bool(result))
}

pub(crate) fn runtime_is_false(value: &Value) -> bool {
    match value {
        Value::Bool(false) => true,
        Value::Null => true,
        Value::Unit => false,
        Value::I8(0) | Value::I16(0) | Value::I32(0) | Value::I64(0) => true,
        Value::U8(0) | Value::U16(0) | Value::U32(0) | Value::U64(0) => true,
        Value::Float { bits, .. } => *bits == 0.0,
        _ => false,
    }
}

pub(crate) fn runtime_null() -> Value {
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quickened_i64_arithmetic_preserves_results_and_errors() {
        assert_eq!(
            runtime_add_int(Value::I64(40), Value::I64(2)).unwrap(),
            Value::I64(42)
        );
        assert_eq!(
            runtime_sub_int(Value::I64(40), Value::I64(2)).unwrap(),
            Value::I64(38)
        );
        assert_eq!(
            runtime_mul_int(Value::I64(6), Value::I64(7)).unwrap(),
            Value::I64(42)
        );
        assert_eq!(
            checked_div_int(Value::I64(-3), Value::I64(2)).unwrap(),
            Value::I64(-2)
        );
        assert!(runtime_add_int(Value::I64(i64::MAX), Value::I64(1)).is_err());
        assert!(checked_div_int(Value::I64(1), Value::I64(0)).is_err());
    }

    #[test]
    fn quickened_integer_helpers_fall_back_for_other_numeric_kinds() {
        assert_eq!(
            runtime_add_int(Value::I8(40), Value::I8(2)).unwrap(),
            Value::I8(42)
        );
        assert_eq!(
            runtime_add_int(
                Value::Float {
                    kind: TypeKind::Fp64,
                    bits: 40.0,
                },
                Value::I64(2),
            )
            .unwrap(),
            Value::Float {
                kind: TypeKind::Fp64,
                bits: 42.0,
            }
        );
    }

    #[test]
    fn quickened_i64_comparison_matches_generic_comparison() {
        for op in [Op::Lt, Op::Lte, Op::Gt, Op::Gte, Op::Eq, Op::Ne] {
            assert_eq!(
                runtime_compare_int(op, Value::I64(3), Value::I64(4)).unwrap(),
                runtime_compare(op, Value::I64(3), Value::I64(4)).unwrap()
            );
        }
    }
}
