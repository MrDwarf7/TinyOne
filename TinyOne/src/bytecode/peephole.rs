use std::collections::HashSet;

use crate::{Function, Instr, Op, Program, floor_div};

pub struct PeepholeOptimizer;

impl PeepholeOptimizer {
    pub fn optimize(program: Program) -> Program {
        Program {
            code: Self::optimize_code(&program.code),
            functions: program
                .functions
                .into_iter()
                .map(|function| Function {
                    code: Self::optimize_code(&function.code),
                    ..function
                })
                .collect(),
            ..program
        }
    }

    fn optimize_code(original: &[Instr]) -> Vec<Instr> {
        let mut code = original.to_vec();
        loop {
            let (optimized, changed) = optimize_pass(&code);
            code = optimized;
            if !changed {
                break;
            }
        }
        code
    }
}

/// Optimize one pass while retaining a total mapping from old instruction
/// offsets to new offsets. A fold may begin at a branch target, but it may not
/// consume a target in its interior. This lets straight-line regions inside
/// functions containing loops and conditionals benefit from constant folding.
fn optimize_pass(code: &[Instr]) -> (Vec<Instr>, bool) {
    let branch_targets: HashSet<usize> = code
        .iter()
        .filter_map(|instr| match instr.op {
            Op::Jump | Op::JumpIfZero => usize::try_from(instr.arg).ok(),
            _ => None,
        })
        .collect();
    let mut old_to_new = vec![0usize; code.len() + 1];
    let mut out = Vec::with_capacity(code.len());
    let mut changed = false;
    let mut i = 0usize;

    while i < code.len() {
        old_to_new[i] = out.len();
        if i + 1 < code.len()
            && code[i].op == Op::PushInt
            && code[i + 1].op == Op::Neg
            && !branch_targets.contains(&(i + 1))
            && let Some(value) = code[i].arg.checked_neg()
        {
            old_to_new[i + 1] = out.len();
            out.push(Instr::new(Op::PushInt, value, 0));
            i += 2;
            changed = true;
            continue;
        }
        if i + 2 < code.len()
            && code[i].op == Op::PushInt
            && code[i + 1].op == Op::PushInt
            && !branch_targets.contains(&(i + 1))
            && !branch_targets.contains(&(i + 2))
        {
            let a = code[i].arg;
            let b = code[i + 1].arg;
            if let Some(folded) = fold_binop(code[i + 2].op, a, b) {
                old_to_new[i + 1] = out.len();
                old_to_new[i + 2] = out.len();
                out.push(folded);
                i += 3;
                changed = true;
                continue;
            }
        }
        out.push(code[i]);
        i += 1;
    }
    old_to_new[code.len()] = out.len();

    if changed {
        for instr in &mut out {
            if matches!(instr.op, Op::Jump | Op::JumpIfZero)
                && let Ok(target) = usize::try_from(instr.arg)
                && let Some(mapped) = old_to_new.get(target)
            {
                instr.arg = *mapped as i64;
            }
        }
    }
    (out, changed)
}

/// Folds a constant binary op into a single push instruction. Comparisons
/// fold to `PUSH_BOOL`, not `PUSH_INT` — the unoptimized `Lt`/`Lte`/`Gt`/
/// `Gte`/`Eq`/`Ne` opcodes all produce `Value::Bool` at runtime
/// (`runtime_compare`), so a constant-folded comparison must match: `1 < 2`
/// and `let a = 1; let b = 2; a < b` need to print identically regardless of
/// whether this optimizer fires.
fn fold_binop(op: Op, a: i64, b: i64) -> Option<Instr> {
    Some(match op {
        Op::Add => Instr::new(Op::PushInt, a.checked_add(b)?, 0),
        Op::Sub => Instr::new(Op::PushInt, a.checked_sub(b)?, 0),
        Op::Mul => Instr::new(Op::PushInt, a.checked_mul(b)?, 0),
        Op::Div if b != 0 => Instr::new(Op::PushInt, floor_div(a, b)?, 0),
        Op::Lt => Instr::new(Op::PushBool, (a < b) as i64, 0),
        Op::Lte => Instr::new(Op::PushBool, (a <= b) as i64, 0),
        Op::Gt => Instr::new(Op::PushBool, (a > b) as i64, 0),
        Op::Gte => Instr::new(Op::PushBool, (a >= b) as i64, 0),
        Op::Eq => Instr::new(Op::PushBool, (a == b) as i64, 0),
        Op::Ne => Instr::new(Op::PushBool, (a != b) as i64, 0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_constants_inside_control_flow_and_remaps_targets() {
        let code = vec![
            Instr::new(Op::PushInt, 2, 0),
            Instr::new(Op::PushInt, 3, 0),
            Instr::new(Op::Add, 0, 0),
            Instr::new(Op::Store, 0, 0),
            Instr::new(Op::Load, 0, 0),
            Instr::new(Op::JumpIfZero, 8, 0),
            Instr::new(Op::PushInt, 1, 0),
            Instr::new(Op::Jump, 4, 0),
            Instr::new(Op::Halt, 0, 0),
        ];

        let optimized = PeepholeOptimizer::optimize_code(&code);
        assert_eq!(optimized.len(), 7);
        assert_eq!(optimized[0], Instr::new(Op::PushInt, 5, 0));
        assert_eq!(optimized[3], Instr::new(Op::JumpIfZero, 6, 0));
        assert_eq!(optimized[5], Instr::new(Op::Jump, 2, 0));
    }

    #[test]
    fn does_not_fold_across_a_branch_target() {
        let code = vec![
            Instr::new(Op::Jump, 1, 0),
            Instr::new(Op::PushInt, 2, 0),
            Instr::new(Op::PushInt, 3, 0),
            Instr::new(Op::Add, 0, 0),
            Instr::new(Op::Halt, 0, 0),
        ];

        let optimized = PeepholeOptimizer::optimize_code(&code);
        assert_eq!(optimized.len(), 3);
        assert_eq!(optimized[0], Instr::new(Op::Jump, 1, 0));
        assert_eq!(optimized[1], Instr::new(Op::PushInt, 5, 0));
    }

    #[test]
    fn minimum_integer_negation_is_left_for_runtime_overflow_handling() {
        let code = vec![
            Instr::new(Op::PushInt, i64::MIN, 0),
            Instr::new(Op::Neg, 0, 0),
        ];
        assert_eq!(PeepholeOptimizer::optimize_code(&code), code);
    }
}
