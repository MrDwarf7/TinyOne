use std::collections::HashMap;

use crate::{BUILTINS, Function, Instr, Op, Program, Result, StructDef, TinyOneError};

const MAX_VERIFIER_STEPS: usize = 10_000_000;
const MAX_STACK_DEPTH: i64 = 65_536;
const MAX_VERIFIER_FUNCTIONS: usize = 4_096;
const MAX_VERIFIER_TOTAL_OPS: usize = 262_144;
const MAX_VERIFIER_SLOT_COUNT: usize = 65_536;
const MAX_VERIFIER_NAMES: usize = 65_536;
const MAX_VERIFIER_STRINGS: usize = 65_536;
const MAX_VERIFIER_FIELDS: usize = 65_536;
const MAX_VERIFIER_STRUCTS: usize = 4_096;
const MAX_VERIFIER_STRUCT_FIELDS: usize = 256;
const MAX_VERIFIER_MODULES: usize = 256;
const MAX_VERIFIER_MODULE_IMPORTS: usize = 4_096;
const MAX_VERIFIER_MODULE_EXPORTS: usize = 4_096;
const MAX_VERIFIER_TEXT_BYTES: usize = 1024 * 1024;

pub struct BytecodeVerifier;

struct VerificationContext<'a> {
    functions: &'a [Function],
    strings: &'a [String],
    structs: &'a [StructDef],
    fields: &'a [String],
    global_slot_count: usize,
}

impl BytecodeVerifier {
    pub fn verify(program: &Program) -> Result<()> {
        Self::verify_program_budget(program)?;
        let context = VerificationContext {
            functions: &program.functions,
            strings: &program.strings,
            structs: &program.structs,
            fields: &program.fields,
            global_slot_count: program.slot_count,
        };
        Self::verify_chunk(
            "main",
            &program.code,
            program.slot_count,
            &context,
            Op::Halt,
        )?;
        for (index, function) in program.functions.iter().enumerate() {
            Self::verify_chunk(
                &format!("function {:?} (index {index})", function.name),
                &function.code,
                function.slot_count,
                &context,
                Op::Return,
            )?;
        }
        Ok(())
    }

    fn verify_program_budget(program: &Program) -> Result<()> {
        reject_over_limit("slot_count", program.slot_count, MAX_VERIFIER_SLOT_COUNT)?;
        verify_string_list("names", &program.names, MAX_VERIFIER_NAMES)?;
        verify_string_list("strings", &program.strings, MAX_VERIFIER_STRINGS)?;
        verify_string_list("fields", &program.fields, MAX_VERIFIER_FIELDS)?;
        reject_over_limit("struct count", program.structs.len(), MAX_VERIFIER_STRUCTS)?;
        reject_over_limit("module count", program.modules.len(), MAX_VERIFIER_MODULES)?;
        if program.functions.len() > MAX_VERIFIER_FUNCTIONS {
            return Err(TinyOneError::compile(format!(
                "Verifier: function count {} exceeds limit {MAX_VERIFIER_FUNCTIONS}",
                program.functions.len()
            )));
        }
        let mut total_ops = program.code.len();
        for function in &program.functions {
            reject_over_limit(
                &format!("function {:?} slot_count", function.name),
                function.slot_count,
                MAX_VERIFIER_SLOT_COUNT,
            )?;
            verify_string_list(
                &format!("function {:?} names", function.name),
                &function.names,
                MAX_VERIFIER_NAMES,
            )?;
            if function.param_count > function.slot_count {
                return Err(TinyOneError::compile(format!(
                    "Verifier: function {:?} has {} parameter(s) but only {} slot(s)",
                    function.name, function.param_count, function.slot_count
                )));
            }
            total_ops = total_ops.checked_add(function.code.len()).ok_or_else(|| {
                TinyOneError::compile("Verifier: total instruction count overflow")
            })?;
        }
        for item in &program.structs {
            verify_string_list(
                &format!("struct {:?} fields", item.name),
                &item.fields,
                MAX_VERIFIER_STRUCT_FIELDS,
            )?;
        }
        for module in &program.modules {
            reject_over_limit(
                &format!("module {:?} imports", module.name),
                module.imports.len(),
                MAX_VERIFIER_MODULE_IMPORTS,
            )?;
            verify_string_list(
                &format!("module {:?} function exports", module.name),
                &module.exported_functions,
                MAX_VERIFIER_MODULE_EXPORTS,
            )?;
            verify_string_list(
                &format!("module {:?} struct exports", module.name),
                &module.exported_structs,
                MAX_VERIFIER_MODULE_EXPORTS,
            )?;
        }
        if total_ops > MAX_VERIFIER_TOTAL_OPS {
            return Err(TinyOneError::compile(format!(
                "Verifier: total instruction count {total_ops} exceeds limit {MAX_VERIFIER_TOTAL_OPS}"
            )));
        }
        Ok(())
    }

    fn verify_chunk(
        chunk_name: &str,
        code: &[Instr],
        slot_count: usize,
        context: &VerificationContext<'_>,
        final_op: Op,
    ) -> Result<()> {
        if code.last().map(|instr| instr.op) != Some(final_op) {
            let got = code
                .last()
                .map(|instr| instr.op.name())
                .unwrap_or("nothing");
            return Err(TinyOneError::compile(format!(
                "Verifier: {chunk_name} must end with {}, got {got}",
                final_op.name()
            )));
        }
        for (pc, instr) in code.iter().copied().enumerate() {
            Self::verify_instruction_operands(chunk_name, code, slot_count, context, pc, instr)?;
        }
        let mut seen: HashMap<usize, i64> = HashMap::new();
        let mut todo = Vec::new();
        visit(&mut seen, &mut todo, code, 0, 0, 0, chunk_name)?;
        let mut steps: usize = 0;
        while let Some((pc, depth)) = todo.pop() {
            steps += 1;
            if steps > MAX_VERIFIER_STEPS {
                return Err(TinyOneError::compile(format!(
                    "Verifier: {chunk_name} exceeded step limit ({MAX_VERIFIER_STEPS})"
                )));
            }
            let instr = code.get(pc).copied().ok_or_else(|| {
                TinyOneError::compile(format!(
                    "Verifier: internal invalid instruction {pc} in {chunk_name}"
                ))
            })?;
            let op = instr.op;
            let arg = instr.arg;
            let arg2 = instr.arg2;
            if matches!(op, Op::Load | Op::Store) && checked_index(arg, slot_count).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid slot {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if op == Op::LoadGlobal && checked_index(arg, context.global_slot_count).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid global slot {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if op == Op::PushString && checked_index(arg, context.strings.len()).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid string index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if matches!(op, Op::GetField | Op::SetField)
                && checked_index(arg, context.fields.len()).is_err()
            {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid field index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            match op {
                Op::Jump => visit(&mut seen, &mut todo, code, arg, depth, pc, chunk_name)?,
                Op::JumpIfZero => {
                    let depth = next_depth(pc, depth, -1, chunk_name)?;
                    visit(&mut seen, &mut todo, code, arg, depth, pc, chunk_name)?;
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        depth,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Call => {
                    let function_index = checked_index(arg, context.functions.len()).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let arg_count = usize::try_from(arg2).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid function arity {arg2} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let function = &context.functions[function_index];
                    if arg_count != function.param_count {
                        return Err(TinyOneError::compile(format!(
                            "Function {:?} expects {} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                            function.name, function.param_count
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::MakeArray => {
                    if arg < 0 {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: negative array arity {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::MakeStruct => {
                    let struct_index = checked_index(arg, context.structs.len()).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid struct index {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let field_count = usize::try_from(arg2).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid struct arity {arg2} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let struct_def = &context.structs[struct_index];
                    let expected = struct_def.fields.len();
                    if field_count != expected {
                        return Err(TinyOneError::compile(format!(
                            "Struct {:?} expects {expected} field value(s), got {arg2} at instruction {pc} in {chunk_name}",
                            struct_def.name
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Builtin => {
                    let builtin_index = checked_index(arg, BUILTINS.len()).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid builtin index {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let builtin = BUILTINS[builtin_index];
                    if arg2 < builtin.min_args as i64 || arg2 > builtin.max_args as i64 {
                        return Err(TinyOneError::compile(format!(
                            "Builtin {:?} expects {}..{} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                            builtin.name, builtin.min_args, builtin.max_args
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Return => {
                    if depth != 1 {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: RETURN in {chunk_name} requires one value, got {depth}"
                        )));
                    }
                }
                Op::Halt => {
                    if depth != 0 {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: HALT in {chunk_name} requires empty stack, got {depth}"
                        )));
                    }
                }
                _ => {
                    let effect = stack_effect(op).ok_or_else(|| {
                        TinyOneError::compile(format!(
                            "Verifier: unknown opcode {op:?} at index {pc} in {chunk_name}"
                        ))
                    })?;
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth(pc, depth, effect, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn verify_instruction_operands(
        chunk_name: &str,
        code: &[Instr],
        slot_count: usize,
        context: &VerificationContext<'_>,
        pc: usize,
        instr: Instr,
    ) -> Result<()> {
        let op = instr.op;
        let arg = instr.arg;
        let arg2 = instr.arg2;
        if matches!(op, Op::Load | Op::Store) && checked_index(arg, slot_count).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid slot {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if op == Op::LoadGlobal && checked_index(arg, context.global_slot_count).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid global slot {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if op == Op::PushString && checked_index(arg, context.strings.len()).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid string index {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if matches!(op, Op::GetField | Op::SetField)
            && checked_index(arg, context.fields.len()).is_err()
        {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid field index {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if matches!(op, Op::Jump | Op::JumpIfZero) && checked_index(arg, code.len()).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: instruction {pc} in {chunk_name} targets {arg}"
            )));
        }
        if op == Op::Call {
            let function_index = checked_index(arg, context.functions.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let arg_count = usize::try_from(arg2).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid function arity {arg2} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let function = &context.functions[function_index];
            if arg_count != function.param_count {
                return Err(TinyOneError::compile(format!(
                    "Function {:?} expects {} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                    function.name, function.param_count
                )));
            }
        }
        if op == Op::MakeArray && arg < 0 {
            return Err(TinyOneError::compile(format!(
                "Verifier: negative array arity {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if op == Op::MakeStruct {
            let struct_index = checked_index(arg, context.structs.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid struct index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let field_count = usize::try_from(arg2).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid struct arity {arg2} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let struct_def = &context.structs[struct_index];
            let expected = struct_def.fields.len();
            if field_count != expected {
                return Err(TinyOneError::compile(format!(
                    "Struct {:?} expects {expected} field value(s), got {arg2} at instruction {pc} in {chunk_name}",
                    struct_def.name
                )));
            }
        }
        if op == Op::Builtin {
            let builtin_index = checked_index(arg, BUILTINS.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid builtin index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let builtin = BUILTINS[builtin_index];
            if arg2 < builtin.min_args as i64 || arg2 > builtin.max_args as i64 {
                return Err(TinyOneError::compile(format!(
                    "Builtin {:?} expects {}..{} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                    builtin.name, builtin.min_args, builtin.max_args
                )));
            }
        }
        Ok(())
    }
}

fn visit(
    seen: &mut HashMap<usize, i64>,
    todo: &mut Vec<(usize, i64)>,
    code: &[Instr],
    pc: i64,
    depth: i64,
    origin: usize,
    chunk_name: &str,
) -> Result<()> {
    let Ok(pc_usize) = usize::try_from(pc) else {
        return Err(TinyOneError::compile(format!(
            "Verifier: instruction {origin} in {chunk_name} targets {pc}"
        )));
    };
    if pc_usize >= code.len() {
        return Err(TinyOneError::compile(format!(
            "Verifier: instruction {origin} in {chunk_name} targets {pc}"
        )));
    }
    let pc = pc_usize;
    if let Some(old_depth) = seen.get(&pc) {
        if *old_depth != depth {
            return Err(TinyOneError::compile(format!(
                "Verifier: stack depth mismatch at instruction {pc} in {chunk_name}: {old_depth} vs {depth}"
            )));
        }
        return Ok(());
    }
    seen.insert(pc, depth);
    todo.push((pc, depth));
    Ok(())
}

fn next_depth(pc: usize, depth: i64, delta: i64, chunk_name: &str) -> Result<i64> {
    let depth = depth.checked_add(delta).ok_or_else(|| {
        TinyOneError::compile(format!(
            "Verifier: stack depth overflow at instruction {pc} in {chunk_name}"
        ))
    })?;
    if depth < 0 {
        return Err(TinyOneError::compile(format!(
            "Verifier: stack underflow at instruction {pc} in {chunk_name}"
        )));
    }
    if depth > MAX_STACK_DEPTH {
        return Err(TinyOneError::compile(format!(
            "Verifier: stack depth {depth} exceeds limit in {chunk_name} at instruction {pc}"
        )));
    }
    Ok(depth)
}

fn next_depth_after_popping_to_one(
    pc: usize,
    depth: i64,
    count: i64,
    chunk_name: &str,
) -> Result<i64> {
    let delta = 1i64.checked_sub(count).ok_or_else(|| {
        TinyOneError::compile(format!(
            "Verifier: stack effect overflow at instruction {pc} in {chunk_name}"
        ))
    })?;
    next_depth(pc, depth, delta, chunk_name)
}

fn next_pc(pc: usize) -> Result<i64> {
    let next = pc
        .checked_add(1)
        .ok_or_else(|| TinyOneError::compile("Verifier: instruction index overflow"))?;
    i64::try_from(next).map_err(|_| TinyOneError::compile("Verifier: instruction index too large"))
}

fn checked_index(index: i64, len: usize) -> Result<usize> {
    if index < 0 {
        return Err(TinyOneError::compile("negative index"));
    }
    let index = usize::try_from(index)
        .map_err(|_| TinyOneError::compile("index is too large for this platform"))?;
    if index >= len {
        return Err(TinyOneError::compile("index out of bounds"));
    }
    Ok(index)
}

fn reject_over_limit(name: &str, got: usize, max: usize) -> Result<()> {
    if got > max {
        return Err(TinyOneError::compile(format!(
            "Verifier: {name} {got} exceeds limit {max}"
        )));
    }
    Ok(())
}

fn verify_string_list(name: &str, values: &[String], max_count: usize) -> Result<()> {
    reject_over_limit(name, values.len(), max_count)?;
    let mut bytes = 0usize;
    for value in values {
        bytes = bytes
            .checked_add(value.len())
            .ok_or_else(|| TinyOneError::compile(format!("Verifier: {name} text overflow")))?;
        reject_over_limit(
            &format!("{name} text bytes"),
            bytes,
            MAX_VERIFIER_TEXT_BYTES,
        )?;
    }
    Ok(())
}

fn stack_effect(op: Op) -> Option<i64> {
    Some(match op {
        Op::PushInt | Op::PushString | Op::PushNull | Op::Load | Op::LoadGlobal => 1,
        Op::Store | Op::Pop => -1,
        Op::Add
        | Op::Sub
        | Op::Mul
        | Op::Div
        | Op::Lt
        | Op::Lte
        | Op::Gt
        | Op::Gte
        | Op::Eq
        | Op::Ne
        | Op::Index => -1,
        Op::Neg | Op::GetField => 0,
        Op::Print => -1,
        Op::SetIndex => -3,
        Op::SetField => -2,
        _ => return None,
    })
}
