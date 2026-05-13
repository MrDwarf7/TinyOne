use crate::{Op, RawPointer, Result, TinyOneError, Value};

pub(crate) fn expect_int(value: &Value, operation: &str) -> Result<i64> {
    match value {
        Value::Int(value) => Ok(*value),
        _ => Err(TinyOneError::runtime(format!(
            "{operation} expects integer operands"
        ))),
    }
}

pub(crate) fn expect_int_pair(lhs: Value, rhs: Value, operation: &str) -> Result<(i64, i64)> {
    Ok((expect_int(&lhs, operation)?, expect_int(&rhs, operation)?))
}

pub(crate) fn runtime_add_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Addition")?;
    Ok(Value::Int(lhs.checked_add(rhs).ok_or_else(|| {
        TinyOneError::runtime("Addition overflow")
    })?))
}

pub(crate) fn runtime_add(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_add_int(lhs, rhs)
}

pub(crate) fn runtime_sub_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Subtraction")?;
    Ok(Value::Int(lhs.checked_sub(rhs).ok_or_else(|| {
        TinyOneError::runtime("Subtraction overflow")
    })?))
}

pub(crate) fn runtime_sub(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_sub_int(lhs, rhs)
}

pub(crate) fn runtime_mul_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Multiplication")?;
    Ok(Value::Int(lhs.checked_mul(rhs).ok_or_else(|| {
        TinyOneError::runtime("Multiplication overflow")
    })?))
}

pub(crate) fn runtime_mul(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_mul_int(lhs, rhs)
}

pub(crate) fn checked_div_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Division")?;
    if rhs == 0 {
        return Err(TinyOneError::runtime("Division by zero"));
    }
    Ok(Value::Int(floor_div(lhs, rhs).ok_or_else(|| {
        TinyOneError::runtime("Division overflow")
    })?))
}

pub(crate) fn checked_div(lhs: Value, rhs: Value) -> Result<Value> {
    checked_div_int(lhs, rhs)
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
    Ok(Value::Int(
        expect_int(&value, "Negation")?
            .checked_neg()
            .ok_or_else(|| TinyOneError::runtime("Negation overflow"))?,
    ))
}

pub(crate) fn runtime_compare_int(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, op.name())?;
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
    Ok(Value::Int(result as i64))
}

pub(crate) fn runtime_compare(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
    runtime_compare_int(op, lhs, rhs)
}

pub(crate) fn runtime_is_false(value: &Value) -> bool {
    matches!(value, Value::Int(0)) || runtime_is_null(value)
}

pub(crate) fn runtime_is_null(value: &Value) -> bool {
    matches!(
        value,
        Value::Pointer(pointer) if pointer.kind == "null" && pointer.address == 0
    )
}

pub(crate) fn runtime_null() -> Value {
    Value::Pointer(RawPointer::new(0, "null", 0, "", 0, ""))
}
