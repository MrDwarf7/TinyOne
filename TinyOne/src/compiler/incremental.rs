use std::collections::{HashMap, HashSet};

use crate::{Instr, Op, Program, Result, TinyOneError, VerifiedProgram};

pub(crate) fn patch_module(
    cached: VerifiedProgram,
    replacement: &Program,
    module_name: &str,
) -> Result<VerifiedProgram> {
    let mut program = cached.into_program();
    let owned_name = |name: &str| {
        name.split_once('.')
            .is_some_and(|(owner, _)| owner == module_name)
    };

    require_same_set(
        "function",
        program
            .functions
            .iter()
            .filter(|item| owned_name(&item.name))
            .map(|item| item.name.as_str()),
        replacement
            .functions
            .iter()
            .filter(|item| owned_name(&item.name))
            .map(|item| item.name.as_str()),
    )?;
    require_same_set(
        "struct",
        program
            .structs
            .iter()
            .filter(|item| owned_name(&item.name))
            .map(|item| item.name.as_str()),
        replacement
            .structs
            .iter()
            .filter(|item| owned_name(&item.name))
            .map(|item| item.name.as_str()),
    )?;

    let function_indexes = program
        .functions
        .iter()
        .enumerate()
        .map(|(index, item)| (item.name.clone(), index))
        .collect::<HashMap<_, _>>();
    let function_map = replacement
        .functions
        .iter()
        .map(|item| {
            function_indexes.get(&item.name).copied().ok_or_else(|| {
                TinyOneError::compile(format!(
                    "Incremental module requires unavailable function {:?}",
                    item.name
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let struct_indexes = program
        .structs
        .iter()
        .enumerate()
        .map(|(index, item)| (item.name.clone(), index))
        .collect::<HashMap<_, _>>();
    let struct_map = replacement
        .structs
        .iter()
        .map(|item| {
            struct_indexes.get(&item.name).copied().ok_or_else(|| {
                TinyOneError::compile(format!(
                    "Incremental module requires unavailable struct {:?}",
                    item.name
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut string_indexes = program
        .strings
        .iter()
        .enumerate()
        .map(|(index, value)| (value.clone(), index))
        .collect::<HashMap<_, _>>();
    let string_map = replacement
        .strings
        .iter()
        .map(|value| intern(value, &mut program.strings, &mut string_indexes))
        .collect::<Vec<_>>();

    let mut field_indexes = program
        .fields
        .iter()
        .enumerate()
        .map(|(index, value)| (value.clone(), index))
        .collect::<HashMap<_, _>>();
    let field_map = replacement
        .fields
        .iter()
        .map(|value| intern(value, &mut program.fields, &mut field_indexes))
        .collect::<Vec<_>>();

    let enum_indexes = program
        .enum_variants
        .iter()
        .enumerate()
        .map(|(index, item)| ((item.enum_name.clone(), item.variant_name.clone()), index))
        .collect::<HashMap<_, _>>();
    let enum_map = replacement
        .enum_variants
        .iter()
        .map(|item| {
            enum_indexes
                .get(&(item.enum_name.clone(), item.variant_name.clone()))
                .copied()
                .ok_or_else(|| {
                    TinyOneError::compile(format!(
                        "Incremental module requires unavailable enum variant {:?}.{:?}",
                        item.enum_name, item.variant_name
                    ))
                })
        })
        .collect::<Result<Vec<_>>>()?;

    for (replacement_index, replacement_function) in replacement.functions.iter().enumerate() {
        if !owned_name(&replacement_function.name) {
            continue;
        }
        let old_index = function_map[replacement_index];
        let mut function = replacement_function.clone();
        remap_code(
            &mut function.code,
            &function_map,
            &string_map,
            &struct_map,
            &field_map,
            &enum_map,
        )?;
        program.functions[old_index] = function;
    }

    for (replacement_index, replacement_struct) in replacement.structs.iter().enumerate() {
        if owned_name(&replacement_struct.name) {
            program.structs[struct_map[replacement_index]] = replacement_struct.clone();
        }
    }

    for (replacement_index, item) in replacement.enum_variants.iter().enumerate() {
        if owned_name(&item.enum_name) {
            program.enum_variants[enum_map[replacement_index]] = item.clone();
        }
    }

    let replacement_module = replacement
        .modules
        .iter()
        .find(|module| module.name == module_name)
        .ok_or_else(|| TinyOneError::compile("Incremental replacement module metadata missing"))?;
    for import in &replacement_module.imports {
        if !program
            .modules
            .iter()
            .any(|module| module.name == import.resolved)
        {
            return Err(TinyOneError::compile(format!(
                "Incremental module introduces unavailable dependency {:?}",
                import.resolved
            )));
        }
    }
    let module_slot = program
        .modules
        .iter_mut()
        .find(|module| module.name == module_name)
        .ok_or_else(|| TinyOneError::compile("Cached module metadata missing"))?;
    *module_slot = replacement_module.clone();
    VerifiedProgram::verify(program)
}

fn require_same_set<'a>(
    kind: &str,
    old: impl Iterator<Item = &'a str>,
    new: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let old = old.collect::<HashSet<_>>();
    let new = new.collect::<HashSet<_>>();
    if old != new {
        return Err(TinyOneError::compile(format!(
            "Incremental module {kind} declarations changed"
        )));
    }
    Ok(())
}

fn intern(value: &str, values: &mut Vec<String>, indexes: &mut HashMap<String, usize>) -> usize {
    if let Some(index) = indexes.get(value) {
        return *index;
    }
    let index = values.len();
    values.push(value.to_string());
    indexes.insert(value.to_string(), index);
    index
}

fn remap_code(
    code: &mut [Instr],
    functions: &[usize],
    strings: &[usize],
    structs: &[usize],
    fields: &[usize],
    enums: &[usize],
) -> Result<()> {
    for instruction in code {
        let mapping = match instruction.op {
            Op::Call | Op::PushFunction => Some(functions),
            Op::PushString => Some(strings),
            Op::MakeStruct => Some(structs),
            Op::GetField | Op::SetField => Some(fields),
            Op::MakeEnum => Some(enums),
            _ => None,
        };
        let Some(mapping) = mapping else {
            continue;
        };
        let old_index = usize::try_from(instruction.arg).map_err(|_| {
            TinyOneError::compile("Incremental module contains a negative table index")
        })?;
        let new_index = mapping.get(old_index).copied().ok_or_else(|| {
            TinyOneError::compile("Incremental module contains an invalid table index")
        })?;
        instruction.arg = i64::try_from(new_index)
            .map_err(|_| TinyOneError::compile("Incremental table index overflow"))?;
    }
    Ok(())
}
