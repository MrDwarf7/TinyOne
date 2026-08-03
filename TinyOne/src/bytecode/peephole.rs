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
        if original
            .iter()
            .any(|instr| matches!(instr.op, Op::Jump | Op::JumpIfZero))
        {
            return original.to_vec();
        }
        let mut code = original.to_vec();
        let mut changed = true;
        while changed {
            changed = false;
            let mut out = Vec::with_capacity(code.len());
            let mut i = 0usize;
            while i < code.len() {
                if i + 1 < code.len() && code[i].op == Op::PushInt && code[i + 1].op == Op::Neg {
                    out.push(Instr::new(Op::PushInt, -code[i].arg, 0));
                    i += 2;
                    changed = true;
                    continue;
                }
                if i + 2 < code.len() && code[i].op == Op::PushInt && code[i + 1].op == Op::PushInt
                {
                    let a = code[i].arg;
                    let b = code[i + 1].arg;
                    if let Some(folded) = fold_binop(code[i + 2].op, a, b) {
                        out.push(folded);
                        i += 3;
                        changed = true;
                        continue;
                    }
                }
                out.push(code[i]);
                i += 1;
            }
            code = out;
        }
        code
    }
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
