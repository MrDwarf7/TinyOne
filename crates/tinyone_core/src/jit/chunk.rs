use std::collections::HashSet;

use crate::{Instr, JitOp, Op, Result, TinyOneError, builtin_index, checked_non_negative_usize};

#[derive(Debug, Clone)]
pub(crate) struct JitChunk {
    pub(crate) name:        String,
    pub(crate) slot_count:  usize,
    pub(crate) ops:         Vec<JitOp>,
    pub(crate) edge_counts: Vec<u16>,
}

impl JitChunk {
    pub(crate) fn compile(name: impl Into<String>, slot_count: usize, code: &[Instr]) -> Result<Self> {
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
        .filter_map(|instr| {
            match instr.op {
                Op::Jump | Op::JumpIfZero => usize::try_from(instr.arg).ok(),
                _ => None,
            }
        })
        .collect()
}

fn superinstruction(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<(JitOp, usize)> {
    if let Some(op) = assign_literal(code, pc, branch_targets) {
        return Some((op, 2));
    }
    if let Some(op) = slot_to_slot_move(code, pc, branch_targets) {
        return Some((op, 2));
    }
    if let Some(op) = slot_immediate_update(code, pc, branch_targets) {
        return Some((op, 4));
    }
    if let Some(op) = slot_immediate_compare_jump(code, pc, branch_targets) {
        return Some((op, 4));
    }
    if let Some(op) = slot_zero_jump(code, pc, branch_targets) {
        return Some((op, 2));
    }
    if let Some(op) = array_len_slot_jump(code, pc, branch_targets) {
        return Some((op, 5));
    }
    if let Some(op) = map_get_add_slots(code, pc, branch_targets) {
        return Some((op, 6));
    }
    if let Some(op) = map_set_mul_slots(code, pc, branch_targets) {
        return Some((op, 7));
    }
    if let Some(op) = slot_immediate_arithmetic(code, pc, branch_targets) {
        return Some((op, 3));
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

fn slot_to_slot_move(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [load, store] = code.get(pc..pc + 2)? else {
        return None;
    };
    if branch_targets.contains(&(pc + 1)) || !matches!(load.op, Op::Load) || !matches!(store.op, Op::Store) {
        return None;
    }
    Some(JitOp::MoveSlot(jit_operand(load.arg).ok()?, jit_operand(store.arg).ok()?))
}

fn slot_immediate_update(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
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
        Op::Mul => Some(JitOp::MulSlotInt(jit_operand(load.arg).ok()?, value.arg)),
        Op::Div => Some(JitOp::DivSlotInt(jit_operand(load.arg).ok()?, value.arg)),
        _ => None,
    }
}

fn slot_immediate_compare_jump(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [load, value, compare, jump] = code.get(pc..pc + 4)? else {
        return None;
    };
    if (pc + 1..pc + 4).any(|target| branch_targets.contains(&target))
        || !matches!(load.op, Op::Load)
        || !matches!(value.op, Op::PushInt)
        || !matches!(compare.op, Op::Lt | Op::Lte | Op::Gt | Op::Gte | Op::Eq | Op::Ne)
        || !matches!(jump.op, Op::JumpIfZero)
    {
        return None;
    }
    JitOp::compare_slot_int_jump_if_zero(
        jit_operand(load.arg).ok()?,
        value.arg,
        compare.op,
        jit_operand(jump.arg).ok()?,
    )
}

fn slot_zero_jump(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [load, jump] = code.get(pc..pc + 2)? else {
        return None;
    };
    if branch_targets.contains(&(pc + 1)) || !matches!(load.op, Op::Load) || !matches!(jump.op, Op::JumpIfZero) {
        return None;
    }
    Some(JitOp::JumpIfZeroSlot(jit_operand(load.arg).ok()?, jit_operand(jump.arg).ok()?))
}

/// Fuses the canonical lowering of `len(array_slot) > 0` used by pop loops.
/// The intermediate instructions must not be a branch target because jumping
/// into the fused range would otherwise skip visible operand-stack effects.
fn array_len_slot_jump(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [load, builtin, zero, compare, jump] = code.get(pc..pc + 5)? else {
        return None;
    };
    if (pc + 1..pc + 5).any(|target| branch_targets.contains(&target))
        || !matches!(load.op, Op::Load)
        || !matches!(builtin.op, Op::Builtin)
        || builtin.arg != builtin_index("len")? as i64
        || builtin.arg2 != 1
        || !matches!(zero.op, Op::PushInt)
        || zero.arg != 0
        || !matches!(compare.op, Op::Gt)
        || !matches!(jump.op, Op::JumpIfZero)
    {
        return None;
    }
    Some(JitOp::ArrayLenSlotJumpIfZero(jit_operand(load.arg).ok()?, jit_operand(jump.arg).ok()?))
}

/// Fuses `total = total + map_get(map, key)` while retaining the direct JIT
/// builtin's numeric-key and generic fallback behavior in the VM executor.
fn map_get_add_slots(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [total, map, key, builtin, add, store] = code.get(pc..pc + 6)? else {
        return None;
    };
    if (pc + 1..pc + 6).any(|target| branch_targets.contains(&target))
        || !matches!(total.op, Op::Load)
        || !matches!(map.op, Op::Load)
        || !matches!(key.op, Op::Load)
        || !matches!(builtin.op, Op::Builtin)
        || builtin.arg != builtin_index("map_get")? as i64
        || builtin.arg2 != 2
        || !matches!(add.op, Op::Add)
        || !matches!(store.op, Op::Store)
        || store.arg != total.arg
    {
        return None;
    }
    JitOp::map_get_add_slots(jit_operand(total.arg).ok()?, jit_operand(map.arg).ok()?, jit_operand(key.arg).ok()?)
}

/// Fuses `ignored = map_set(map, key, value * K)` into one lowered operation.
/// The executor uses the I64 fast path only when the dynamic slots actually
/// hold I64 values and preserves the checked generic arithmetic otherwise.
fn map_set_mul_slots(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [map, key, value, multiplier, multiply, builtin, store] = code.get(pc..pc + 7)? else {
        return None;
    };
    if (pc + 1..pc + 7).any(|target| branch_targets.contains(&target))
        || !matches!(map.op, Op::Load)
        || !matches!(key.op, Op::Load)
        || !matches!(value.op, Op::Load)
        || !matches!(multiplier.op, Op::PushInt)
        || !matches!(multiply.op, Op::Mul)
        || !matches!(builtin.op, Op::Builtin)
        || builtin.arg != builtin_index("map_set")? as i64
        || builtin.arg2 != 3
        || !matches!(store.op, Op::Store)
    {
        return None;
    }
    JitOp::map_set_mul_slots(
        jit_operand(map.arg).ok()?,
        jit_operand(key.arg).ok()?,
        jit_operand(value.arg).ok()?,
        jit_operand(store.arg).ok()?,
        multiplier.arg,
    )
}

fn slot_immediate_arithmetic(code: &[Instr], pc: usize, branch_targets: &HashSet<usize>) -> Option<JitOp> {
    let [load, value, op] = code.get(pc..pc + 3)? else {
        return None;
    };
    if (pc + 1..pc + 3).any(|target| branch_targets.contains(&target))
        || !matches!(load.op, Op::Load)
        || !matches!(value.op, Op::PushInt)
    {
        return None;
    }
    let slot = jit_operand(load.arg).ok()?;
    match op.op {
        Op::Mul => Some(JitOp::PushMulSlotInt(slot, value.arg)),
        Op::Div => Some(JitOp::PushDivSlotInt(slot, value.arg)),
        _ => None,
    }
}

fn jit_operand(value: i64) -> Result<usize> {
    checked_non_negative_usize(value, "JIT operand")
        .map_err(|error| TinyOneError::compile(format!("JIT invalid operand: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuses_slot_immediate_compare_and_conditional_jump() {
        for compare in [Op::Lt, Op::Lte, Op::Gt, Op::Gte, Op::Eq, Op::Ne] {
            let code = [
                Instr::new(Op::Load, 0, 0),
                Instr::new(Op::PushInt, 10, 0),
                Instr::new(compare, 0, 0),
                Instr::new(Op::JumpIfZero, 5, 0),
                Instr::new(Op::PushInt, 1, 0),
                Instr::new(Op::Halt, 0, 0),
            ];

            let ops = compile_ops(&code).unwrap();
            assert_eq!(ops.len(), 3);
            assert_eq!(ops[0].listing(), format!("slot.cmp.{}.i.jz 0 10 2", compare.name().to_ascii_lowercase()));
        }
    }

    #[test]
    fn branch_target_inside_compare_sequence_prevents_fusion() {
        let code = [
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::PushInt, 10, 0),
            Instr::new(Op::Lt, 0, 0),
            Instr::new(Op::JumpIfZero, 5, 0),
            Instr::new(Op::Halt, 0, 0),
        ];

        let ops = compile_ops(&code).unwrap();
        assert!(matches!(ops[1], JitOp::Load(0)));
        assert!(matches!(ops[2], JitOp::PushInt(10)));
        assert!(matches!(ops[3], JitOp::Compare(Op::Lt)));
        assert!(matches!(ops[4], JitOp::JumpIfZero(5)));
    }

    #[test]
    fn fuses_slot_immediate_multiply_and_divide() {
        for (op, expected) in [
            (Op::Mul, JitOp::PushMulSlotInt(0, -3)),
            (Op::Div, JitOp::PushDivSlotInt(0, -3)),
        ] {
            let code = [
                Instr::new(Op::Load, 0, 0),
                Instr::new(Op::PushInt, -3, 0),
                Instr::new(op, 0, 0),
                Instr::new(Op::Print, 0, 0),
                Instr::new(Op::Halt, 0, 0),
            ];
            let ops = compile_ops(&code).unwrap();
            assert_eq!(ops.len(), 3);
            assert_eq!(ops[0], expected);
        }
    }

    #[test]
    fn fuses_slot_move_and_in_place_multiply_and_divide() {
        let move_code = [
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::Store, 1, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        assert_eq!(compile_ops(&move_code).unwrap()[0], JitOp::MoveSlot(0, 1));

        for (op, expected) in [(Op::Mul, JitOp::MulSlotInt(0, -3)), (Op::Div, JitOp::DivSlotInt(0, -3))] {
            let code = [
                Instr::new(Op::Load, 0, 0),
                Instr::new(Op::PushInt, -3, 0),
                Instr::new(op, 0, 0),
                Instr::new(Op::Store, 0, 0),
                Instr::new(Op::Halt, 0, 0),
            ];
            assert_eq!(compile_ops(&code).unwrap()[0], expected);
        }
    }

    #[test]
    fn fuses_slot_zero_jump_and_remaps_its_target() {
        let code = [
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::JumpIfZero, 3, 0),
            Instr::new(Op::PushInt, 1, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let ops = compile_ops(&code).unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0], JitOp::JumpIfZeroSlot(0, 2));
    }

    #[test]
    fn fuses_array_length_loop_guard_without_exposing_internal_targets() {
        let len = builtin_index("len").expect("known builtin") as i64;
        let code = [
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::Builtin, len, 1),
            Instr::new(Op::PushInt, 0, 0),
            Instr::new(Op::Gt, 0, 0),
            Instr::new(Op::JumpIfZero, 6, 0),
            Instr::new(Op::Halt, 0, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let ops = compile_ops(&code).unwrap();
        assert_eq!(ops[0], JitOp::ArrayLenSlotJumpIfZero(0, 2));

        let guarded = [
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::Builtin, len, 1),
            Instr::new(Op::PushInt, 0, 0),
            Instr::new(Op::Gt, 0, 0),
            Instr::new(Op::JumpIfZero, 6, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let guarded_ops = compile_ops(&guarded).unwrap();
        assert!(matches!(guarded_ops[1], JitOp::Load(0)));
    }

    #[test]
    fn fuses_map_get_add_store_for_local_i64_loop_state() {
        let map_get = builtin_index("map_get").expect("known builtin") as i64;
        let code = [
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::Load, 1, 0),
            Instr::new(Op::Load, 2, 0),
            Instr::new(Op::Builtin, map_get, 2),
            Instr::new(Op::Add, 0, 0),
            Instr::new(Op::Store, 0, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let ops = compile_ops(&code).unwrap();
        assert_eq!(ops[0].listing(), "map.get.add.slots 0 1 2");

        let guarded = [
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::Load, 1, 0),
            Instr::new(Op::Load, 2, 0),
            Instr::new(Op::Builtin, map_get, 2),
            Instr::new(Op::Add, 0, 0),
            Instr::new(Op::Store, 0, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let guarded_ops = compile_ops(&guarded).unwrap();
        assert!(matches!(guarded_ops[1], JitOp::Load(0)));
    }

    #[test]
    fn fuses_map_set_with_slot_immediate_product() {
        let map_set = builtin_index("map_set").expect("known builtin") as i64;
        let code = [
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::Load, 1, 0),
            Instr::new(Op::Load, 1, 0),
            Instr::new(Op::PushInt, 3, 0),
            Instr::new(Op::Mul, 0, 0),
            Instr::new(Op::Builtin, map_set, 3),
            Instr::new(Op::Store, 2, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let ops = compile_ops(&code).unwrap();
        assert_eq!(ops[0].listing(), "map.set.mul.slots 0 1 1 2 3");
    }

    #[test]
    fn branch_target_inside_slot_move_or_update_prevents_fusion() {
        let move_code = [
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::Store, 1, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let move_ops = compile_ops(&move_code).unwrap();
        assert!(matches!(move_ops[1], JitOp::Load(0)));
        assert!(matches!(move_ops[2], JitOp::Store(1)));

        let update_code = [
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::PushInt, 3, 0),
            Instr::new(Op::Mul, 0, 0),
            Instr::new(Op::Store, 0, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let update_ops = compile_ops(&update_code).unwrap();
        assert!(matches!(update_ops[1], JitOp::Load(0)));
        assert!(matches!(update_ops[2], JitOp::PushInt(3)));
        assert!(matches!(update_ops[3], JitOp::Mul));
        assert!(matches!(update_ops[4], JitOp::Store(0)));

        let zero_jump_code = [
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::JumpIfZero, 3, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let zero_jump_ops = compile_ops(&zero_jump_code).unwrap();
        assert!(matches!(zero_jump_ops[1], JitOp::Load(0)));
        assert!(matches!(zero_jump_ops[2], JitOp::JumpIfZero(3)));
    }

    #[test]
    fn branch_target_inside_slot_arithmetic_prevents_fusion() {
        let code = [
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::PushInt, 3, 0),
            Instr::new(Op::Mul, 0, 0),
            Instr::new(Op::Halt, 0, 0),
        ];
        let ops = compile_ops(&code).unwrap();
        assert!(matches!(ops[1], JitOp::Load(0)));
        assert!(matches!(ops[2], JitOp::PushInt(3)));
        assert!(matches!(ops[3], JitOp::Mul));
    }
}
