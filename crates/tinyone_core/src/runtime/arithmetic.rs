use crate::{Op, Result, TinyOneError, TypeKind, Value};

pub(crate) fn expect_int(value: &Value, operation: &str) -> Result<i64> {
    let raw = runtime_integer_value(value, operation)?;
    i64::try_from(raw).map_err(|_| TinyOneError::runtime(format!("{operation} integer value is too large for i64")))
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
        _ => Err(TinyOneError::runtime(format!("{operation} expects integer operands"))),
    }
}

pub(crate) fn runtime_cast_int(value: &Value, kind: TypeKind, operation: &str) -> Result<Value> {
    let value = runtime_integer_value(value, operation)?;
    integer_value_from_kind(kind, value, operation)
}

pub(crate) fn integer_value_from_kind(kind: TypeKind, value: i128, operation: &str) -> Result<Value> {
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

pub(crate) fn runtime_add_int(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_add(lhs, rhs)
}

pub(crate) fn runtime_add(lhs: Value, rhs: Value) -> Result<Value> {
    let kind = arithmetic_kind(&lhs, &rhs, "Addition")?;
    let lhs = runtime_integer_value(&lhs, "Addition")?;
    let rhs = runtime_integer_value(&rhs, "Addition")?;
    let result = lhs
        .checked_add(rhs)
        .ok_or_else(|| TinyOneError::runtime("Addition overflow"))?;
    integer_value_from_kind(kind, result, "Addition")
}

pub(crate) fn runtime_sub_int(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_sub(lhs, rhs)
}

pub(crate) fn runtime_sub(lhs: Value, rhs: Value) -> Result<Value> {
    let kind = arithmetic_kind(&lhs, &rhs, "Subtraction")?;
    let lhs = runtime_integer_value(&lhs, "Subtraction")?;
    let rhs = runtime_integer_value(&rhs, "Subtraction")?;
    let result = lhs
        .checked_sub(rhs)
        .ok_or_else(|| TinyOneError::runtime("Subtraction overflow"))?;
    integer_value_from_kind(kind, result, "Subtraction")
}

pub(crate) fn runtime_mul_int(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_mul(lhs, rhs)
}

pub(crate) fn runtime_mul(lhs: Value, rhs: Value) -> Result<Value> {
    let kind = arithmetic_kind(&lhs, &rhs, "Multiplication")?;
    let lhs = runtime_integer_value(&lhs, "Multiplication")?;
    let rhs = runtime_integer_value(&rhs, "Multiplication")?;
    let result = lhs
        .checked_mul(rhs)
        .ok_or_else(|| TinyOneError::runtime("Multiplication overflow"))?;
    integer_value_from_kind(kind, result, "Multiplication")
}

pub(crate) fn checked_div_int(lhs: Value, rhs: Value) -> Result<Value> {
    checked_div(lhs, rhs)
}

pub(crate) fn checked_div(lhs: Value, rhs: Value) -> Result<Value> {
    let kind = arithmetic_kind(&lhs, &rhs, "Division")?;
    let lhs_value = runtime_integer_value(&lhs, "Division")?;
    let rhs_value = runtime_integer_value(&rhs, "Division")?;
    if rhs_value == 0 {
        return Err(TinyOneError::runtime("Division by zero"));
    }
    let result = if kind == TypeKind::I64 {
        let lhs = i64::try_from(lhs_value).map_err(|_| TinyOneError::runtime("Division left operand is too large"))?;
        let rhs = i64::try_from(rhs_value).map_err(|_| TinyOneError::runtime("Division right operand is too large"))?;
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
        return Err(TinyOneError::runtime(format!("{operation} must be non-negative")));
    }
    usize::try_from(value).map_err(|_| TinyOneError::runtime(format!("{operation} is too large")))
}

pub(crate) fn checked_bounded_len(value: i64, operation: &str, max: usize) -> Result<usize> {
    let value = checked_non_negative_usize(value, operation)?;
    if value > max {
        return Err(TinyOneError::runtime(format!("{operation} exceeds maximum length {max}")));
    }
    Ok(value)
}

pub(crate) fn checked_collection_index(index: i64, len: usize, kind: &str) -> Result<usize> {
    if index < 0 {
        return Err(TinyOneError::runtime(format!("{kind} index {index} out of bounds")));
    }
    let index =
        usize::try_from(index).map_err(|_| TinyOneError::runtime(format!("{kind} index {index} is too large")))?;
    if index >= len {
        return Err(TinyOneError::runtime(format!("{kind} index {index} out of bounds")));
    }
    Ok(index)
}

pub(crate) fn checked_byte_range(offset: i64, width: usize, len: usize, operation: &str) -> Result<usize> {
    if offset < 0 {
        return Err(TinyOneError::runtime(format!("{operation} out of bounds at byte offset {offset}")));
    }
    let offset = usize::try_from(offset)
        .map_err(|_| TinyOneError::runtime(format!("{operation} byte offset {offset} is too large")))?;
    let end = offset
        .checked_add(width)
        .ok_or_else(|| TinyOneError::runtime(format!("{operation} byte range overflows at offset {offset}")))?;
    if end > len {
        return Err(TinyOneError::runtime(format!("{operation} out of bounds at byte offset {offset}")));
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
    let kind =
        runtime_integer_kind(&value).ok_or_else(|| TinyOneError::runtime("Negation expects integer operands"))?;
    if kind.is_unsigned() {
        return Err(TinyOneError::runtime("Negation expects a signed integer operand"));
    }
    let value = runtime_integer_value(&value, "Negation")?;
    let result = value
        .checked_neg()
        .ok_or_else(|| TinyOneError::runtime("Negation overflow"))?;
    integer_value_from_kind(kind, result, "Negation")
}

pub(crate) fn runtime_compare_int(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
    runtime_compare(op, lhs, rhs)
}

pub(crate) fn runtime_compare(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
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
            return Err(TinyOneError::runtime(format!("Unsupported comparison opcode {op:?}")));
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
        Value::Bf16(0) => true,
        Value::Float { bits, .. } => *bits == 0.0,
        _ => false,
    }
}

pub(crate) fn runtime_is_null(value: &Value) -> bool {
    matches!(value, Value::Null) || matches!(value, Value::Pointer(p) if p.kind == "null" && p.address == 0)
}

pub(crate) fn runtime_null() -> Value {
    Value::Null
}
