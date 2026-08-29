use crate::Op;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instr {
    pub op:   Op,
    pub arg:  i64,
    pub arg2: i64,
}

impl Instr {
    pub fn new(op: Op, arg: i64, arg2: i64) -> Self {
        Self { op, arg, arg2 }
    }
}
