use crate::{
    HeapData, MAX_ARRAY_LENGTH, Result, TinyOneError, TinyRuntimeContext, Value,
    checked_collection_index, expect_int,
};

pub(crate) fn runtime_make_array(
    context: &mut TinyRuntimeContext,
    values: Vec<Value>,
) -> Result<Value> {
    if values.len() > MAX_ARRAY_LENGTH {
        return Err(TinyOneError::runtime(format!(
            "array literal exceeds maximum length {MAX_ARRAY_LENGTH}"
        )));
    }
    Ok(Value::Heap(context.heap().alloc_array(values)?))
}

pub(crate) fn runtime_index(
    context: &mut TinyRuntimeContext,
    container: Value,
    index: Value,
) -> Result<Value> {
    let index = expect_int(&index, "Index")?;
    let object = context.heap().get(&container)?.clone();
    match object.data {
        HeapData::Array(values) => {
            let index = checked_collection_index(index, values.len(), "Array")?;
            values
                .get(index)
                .cloned()
                .ok_or_else(|| TinyOneError::runtime("Array index out of bounds"))
        }
        HeapData::String(text) => {
            let index = checked_collection_index(index, text.chars().count(), "String")?;
            let ch = text
                .chars()
                .nth(index)
                .ok_or_else(|| TinyOneError::runtime("String index out of bounds"))?;
            Ok(Value::Heap(context.heap().alloc_string(ch.to_string())?))
        }
        _ => Err(TinyOneError::runtime(format!(
            "Cannot index {}",
            object.kind()
        ))),
    }
}

pub(crate) fn runtime_set_index(
    context: &mut TinyRuntimeContext,
    container: Value,
    index: Value,
    value: Value,
) -> Result<()> {
    let index = expect_int(&index, "Index")?;
    let mut heap = context.heap();
    let object = heap.get_mut(&container)?;
    let kind = object.kind();
    let HeapData::Array(values) = &mut object.data else {
        return Err(TinyOneError::runtime(format!(
            "Cannot assign index on {kind}"
        )));
    };
    let index = checked_collection_index(index, values.len(), "Array")?;
    let target = values
        .get_mut(index)
        .ok_or_else(|| TinyOneError::runtime("Array index out of bounds"))?;
    *target = value;
    Ok(())
}

pub(crate) fn runtime_array_push(
    context: &mut TinyRuntimeContext,
    target: &Value,
    value: Value,
) -> Result<Value> {
    Ok(Value::Int(context.heap().grow_array(target, value)? as i64))
}

pub(crate) fn runtime_array_pop(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    context.heap().shrink_array(target)
}

pub(crate) fn runtime_make_struct(
    context: &mut TinyRuntimeContext,
    type_name: &str,
    field_names: &[String],
    values: Vec<Value>,
) -> Result<Value> {
    let fields = field_names.iter().cloned().zip(values).collect();
    Ok(Value::Heap(context.heap().alloc_struct(type_name, fields)?))
}

pub(crate) fn runtime_get_field(
    context: &TinyRuntimeContext,
    target: Value,
    field: &str,
) -> Result<Value> {
    let heap = context.heap();
    let object = heap.get(&target)?;
    let HeapData::Struct(fields) = &object.data else {
        return Err(TinyOneError::runtime(format!(
            "Cannot read field {field:?} from {}",
            object.kind()
        )));
    };
    fields
        .iter()
        .find(|(name, _)| name == field)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| {
            TinyOneError::runtime(format!(
                "Unknown field {field:?} on struct {:?}",
                object.type_name
            ))
        })
}

pub(crate) fn runtime_set_field(
    context: &mut TinyRuntimeContext,
    target: Value,
    field: &str,
    value: Value,
) -> Result<()> {
    let mut heap = context.heap();
    let object = heap.get_mut(&target)?;
    let type_name = object.type_name.clone();
    let kind = object.kind();
    let HeapData::Struct(fields) = &mut object.data else {
        return Err(TinyOneError::runtime(format!(
            "Cannot write field {field:?} on {kind}"
        )));
    };
    if let Some((_, field_value)) = fields.iter_mut().find(|(name, _)| name == field) {
        *field_value = value;
        Ok(())
    } else {
        Err(TinyOneError::runtime(format!(
            "Unknown field {field:?} on struct {type_name:?}"
        )))
    }
}

pub(crate) fn expect_string(
    context: &TinyRuntimeContext,
    value: &Value,
    operation: &str,
) -> Result<String> {
    let heap = context.heap();
    let object = heap.get(value)?;
    match &object.data {
        HeapData::String(text) => Ok(text.clone()),
        _ => Err(TinyOneError::runtime(format!(
            "{operation} expects a string"
        ))),
    }
}
