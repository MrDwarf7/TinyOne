use std::collections::HashSet;

use crate::{Instr, JitOp, Op, Result, TinyOneError, checked_non_negative_usize};

pub(crate) const HOT_BACK_EDGE_THRESHOLD: u16 = 8;

#[derive(Debug, Clone)]
pub(crate) struct JitChunk {
    pub(crate) name: String,
    pub(crate) slot_count: usize,
    pub(crate) ops: Vec<JitOp>,
    pub(crate) edge_counts: Vec<u16>,
}

impl JitChunk {
    pub(crate) fn compile(
        name: impl Into<String>,
        slot_count: usize,
        code: &[Instr],
    ) -> Result<Self> {
        let ops = compile_ops(code)?;
        Ok(Self {
            name: name.into(),
            slot_count,
            edge_counts: vec![0; ops.len()],
            ops,
        })
    }

    pub(crate) fn promote_range(&mut self, start: usize, end: usize) -> usize {
        let start = start.min(self.ops.len());
        let end = end.min(self.ops.len());
        let mut changed = 0usize;
        for op in &mut self.ops[start..end] {
            let quickened = op.quickened();
            if quickened != *op {
                *op = quickened;
                changed += 1;
            }
        }
        changed
    }
}

fn compile_ops(code: &[Instr]) -> Result<Vec<JitOp>> {
    let branch_targets = branch_targets(code);
    let mut original_to_compiled = vec![0usize; code.len() + 1];
    let mut ops = Vec::with_capacity(code.len());
    let mut pc = 0usize;

    while pc < code.len() {
        original_to_compiled[pc] = ops.len();
        if let Some((op, width)) = superinstruction(code, pc, &branch_targets) {
            for offset in 1..width {
                original_to_compiled[pc + offset] = ops.len();
            }
            ops.push(op);
            pc += width;
            continue;
        }
        ops.push(JitOp::from_instr(code[pc])?);
        pc += 1;
    }
    original_to_compiled[code.len()] = ops.len();

    for op in &mut ops {
        op.remap_targets(&original_to_compiled);
    }
    Ok(ops)
}

fn branch_targets(code: &[Instr]) -> HashSet<usize> {
    code.iter()
        .filter_map(|instr| match instr.op {
            Op::Jump | Op::JumpIfZero => usize::try_from(instr.arg).ok(),
            _ => None,
        })
        .collect()
}

fn superinstruction(
    code: &[Instr],
    pc: usize,
    branch_targets: &HashSet<usize>,
) -> Option<(JitOp, usize)> {
    if let Some(op) = assign_literal(code, pc, branch_targets) {
        return Some((op, 2));
    }
    if let Some(op) = slot_immediate_update(code, pc, branch_targets) {
        return Some((op, 4));
    }
    None
}

fn assign_literal(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [first, second] = code.get(pc..pc + 2)? else {
        return None;
    };
    if branch_targets.contains(&(pc + 1)) {
        return None;
    }
    if matches!(first.op, Op::PushInt) && matches!(second.op, Op::Store) {
        let slot = jit_operand(second.arg).ok()?;
        return Some(JitOp::StoreInt(slot, first.arg));
    }
    None
}

fn slot_immediate_update(
    code: &[Instr],
    pc: usize,
    branch_targets: &HashSet<usize>,
) -> Option<JitOp> {
    let [load, value, op, store] = code.get(pc..pc + 4)? else {
        return None;
    };
    if (pc + 1..pc + 4).any(|target| branch_targets.contains(&target)) {
        return None;
    }
    if !matches!(load.op, Op::Load)
        || !matches!(value.op, Op::PushInt)
        || !matches!(store.op, Op::Store)
        || load.arg != store.arg
    {
        return None;
    }
    match op.op {
        Op::Add => Some(JitOp::AddSlotInt(jit_operand(load.arg).ok()?, value.arg)),
        Op::Sub => Some(JitOp::SubSlotInt(jit_operand(load.arg).ok()?, value.arg)),
        _ => None,
    }
}

fn jit_operand(value: i64) -> Result<usize> {
    checked_non_negative_usize(value, "JIT operand")
        .map_err(|error| TinyOneError::compile(format!("JIT invalid operand: {error}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JitFunction {
    pub(crate) name: String,
    pub(crate) param_count: usize,
    pub(crate) slot_count: usize,
    pub(crate) chunk_index: usize,
}
