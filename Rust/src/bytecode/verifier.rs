use std::collections::HashMap;

use crate::{BUILTINS, Function, Instr, Op, Program, Result, StructDef, TinyOneError};

pub struct BytecodeVerifier;

impl BytecodeVerifier {
    pub fn verify(program: &Program) -> Result<()> {
        Self::verify_chunk(
            "main",
            &program.code,
            program.slot_count,
            &program.functions,
            &program.strings,
            &program.structs,
            &program.fields,
            Op::Halt,
        )?;
        for (index, function) in program.functions.iter().enumerate() {
            Self::verify_chunk(
                &format!("function {:?} (index {index})", function.name),
                &function.code,
                function.slot_count,
                &program.functions,
                &program.strings,
                &program.structs,
                &program.fields,
                Op::Return,
            )?;
        }
        Ok(())
    }

    fn verify_chunk(
        chunk_name: &str,
        code: &[Instr],
        slot_count: usize,
        functions: &[Function],
        strings: &[String],
        structs: &[StructDef],
        fields: &[String],
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
        let mut seen: HashMap<usize, i64> = HashMap::new();
        let mut todo = Vec::new();
        visit(&mut seen, &mut todo, code, 0, 0, 0, chunk_name)?;
        while let Some((pc, depth)) = todo.pop() {
            let instr = code[pc];
            let op = instr.op;
            let arg = instr.arg;
            let arg2 = instr.arg2;
            if matches!(op, Op::Load | Op::Store) && !valid_index(arg, slot_count) {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid slot {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if op == Op::PushString && !valid_index(arg, strings.len()) {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid string index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if matches!(op, Op::GetField | Op::SetField) && !valid_index(arg, fields.len()) {
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
                        (pc + 1) as i64,
                        depth,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Call => {
                    if !valid_index(arg, functions.len()) {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    let function = &functions[arg as usize];
                    if arg2 as usize != function.param_count {
                        return Err(TinyOneError::compile(format!(
                            "Function {:?} expects {} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                            function.name, function.param_count
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg2, chunk_name)?,
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
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::MakeStruct => {
                    if !valid_index(arg, structs.len()) {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: invalid struct index {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    let expected = structs[arg as usize].fields.len();
                    if arg2 as usize != expected {
                        return Err(TinyOneError::compile(format!(
                            "Struct {:?} expects {expected} field value(s), got {arg2} at instruction {pc} in {chunk_name}",
                            structs[arg as usize].name
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Builtin => {
                    if !valid_index(arg, BUILTINS.len()) {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: invalid builtin index {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    let builtin = BUILTINS[arg as usize];
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
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg2, chunk_name)?,
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
                        (pc + 1) as i64,
                        next_depth(pc, depth, effect, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
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
    if pc < 0 || pc as usize >= code.len() {
        return Err(TinyOneError::compile(format!(
            "Verifier: instruction {origin} in {chunk_name} targets {pc}"
        )));
    }
    let pc = pc as usize;
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
    let depth = depth + delta;
    if depth < 0 {
        return Err(TinyOneError::compile(format!(
            "Verifier: stack underflow at instruction {pc} in {chunk_name}"
        )));
    }
    Ok(depth)
}

fn valid_index(index: i64, len: usize) -> bool {
    index >= 0 && (index as usize) < len
}

fn stack_effect(op: Op) -> Option<i64> {
    Some(match op {
        Op::PushInt | Op::PushString | Op::PushNull | Op::Load => 1,
        Op::Store => -1,
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
