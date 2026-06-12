use crate::{Function, Instr, Op, Program, floor_div};

pub struct PeepholeOptimizer;

impl PeepholeOptimizer {
    pub fn optimize(program: Program) -> Program {
        Program {
            code: Self::optimize_code(&program.code),
            functions: program
                .functions
                .into_iter()
                .map(|function| {
                    Function {
                        code: Self::optimize_code(&function.code),
                        ..function
                    }
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
                if i + 2 < code.len() && code[i].op == Op::PushInt && code[i + 1].op == Op::PushInt {
                    let a = code[i].arg;
                    let b = code[i + 1].arg;
                    if let Some(value) = fold_binop(code[i + 2].op, a, b) {
                        out.push(Instr::new(Op::PushInt, value, 0));
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

fn fold_binop(op: Op, a: i64, b: i64) -> Option<i64> {
    Some(match op {
        Op::Add => a.checked_add(b)?,
        Op::Sub => a.checked_sub(b)?,
        Op::Mul => a.checked_mul(b)?,
        Op::Div if b != 0 => floor_div(a, b)?,
        Op::Lt => (a < b) as i64,
        Op::Lte => (a <= b) as i64,
        Op::Gt => (a > b) as i64,
        Op::Gte => (a >= b) as i64,
        Op::Eq => (a == b) as i64,
        Op::Ne => (a != b) as i64,
        _ => return None,
    })
}
