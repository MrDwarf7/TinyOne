use crate::{Instr, Op, Result, TinyOneError, checked_non_negative_usize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JitOp {
    PushInt(i64),
    PushNull,
    Pop,
    PushString(usize),
    Load(usize),
    LoadGlobal(usize),
    Store(usize),
    StoreInt(usize, i64),
    AddSlotInt(usize, i64),
    SubSlotInt(usize, i64),
    Add,
    AddInt,
    Sub,
    SubInt,
    Mul,
    MulInt,
    Div,
    DivInt,
    Neg,
    Compare(Op),
    CompareInt(Op),
    Jump(usize),
    JumpHot(usize),
    JumpIfZero(usize),
    JumpIfZeroHot(usize),
    Call(usize, usize),
    MakeArray(usize),
    Index,
    SetIndex,
    MakeStruct(usize, usize),
    GetField(usize),
    SetField(usize),
    Builtin(usize, usize),
    Return,
    Print,
    Halt,
}

impl JitOp {
    pub(crate) fn from_instr(instr: Instr) -> Result<Self> {
        Ok(match instr.op {
            Op::PushInt => Self::PushInt(instr.arg),
            Op::PushNull => Self::PushNull,
            Op::Pop => Self::Pop,
            Op::PushString => Self::PushString(jit_operand(instr.arg, "string index")?),
            Op::Load => Self::Load(jit_operand(instr.arg, "load slot")?),
            Op::LoadGlobal => Self::LoadGlobal(jit_operand(instr.arg, "global load slot")?),
            Op::Store => Self::Store(jit_operand(instr.arg, "store slot")?),
            Op::Add => Self::Add,
            Op::Sub => Self::Sub,
            Op::Mul => Self::Mul,
            Op::Div => Self::Div,
            Op::Neg => Self::Neg,
            Op::Lt | Op::Lte | Op::Gt | Op::Gte | Op::Eq | Op::Ne => Self::Compare(instr.op),
            Op::Jump => Self::Jump(jit_operand(instr.arg, "jump target")?),
            Op::JumpIfZero => Self::JumpIfZero(jit_operand(instr.arg, "jump target")?),
            Op::Call => Self::Call(
                jit_operand(instr.arg, "function index")?,
                jit_operand(instr.arg2, "function arity")?,
            ),
            Op::MakeArray => Self::MakeArray(jit_operand(instr.arg, "array arity")?),
            Op::Index => Self::Index,
            Op::SetIndex => Self::SetIndex,
            Op::MakeStruct => Self::MakeStruct(
                jit_operand(instr.arg, "struct index")?,
                jit_operand(instr.arg2, "struct arity")?,
            ),
            Op::GetField => Self::GetField(jit_operand(instr.arg, "field index")?),
            Op::SetField => Self::SetField(jit_operand(instr.arg, "field index")?),
            Op::Builtin => Self::Builtin(
                jit_operand(instr.arg, "builtin index")?,
                jit_operand(instr.arg2, "builtin arity")?,
            ),
            Op::Return => Self::Return,
            Op::Print => Self::Print,
            Op::Halt => Self::Halt,
        })
    }

    pub(crate) fn quickened(self) -> Self {
        match self {
            Self::Add => Self::AddInt,
            Self::Sub => Self::SubInt,
            Self::Mul => Self::MulInt,
            Self::Div => Self::DivInt,
            Self::Compare(op) => Self::CompareInt(op),
            Self::Jump(target) => Self::JumpHot(target),
            Self::JumpIfZero(target) => Self::JumpIfZeroHot(target),
            _ => self,
        }
    }

    pub(crate) fn listing(self) -> String {
        match self {
            Self::PushInt(value) => format!("push.i {value}"),
            Self::PushNull => "push.null".to_string(),
            Self::Pop => "pop".to_string(),
            Self::PushString(index) => format!("push.str {index}"),
            Self::Load(slot) => format!("load {slot}"),
            Self::LoadGlobal(slot) => format!("load.global {slot}"),
            Self::Store(slot) => format!("store {slot}"),
            Self::StoreInt(slot, value) => format!("store.i {slot} {value}"),
            Self::AddSlotInt(slot, value) => format!("slot.add.i {slot} {value}"),
            Self::SubSlotInt(slot, value) => format!("slot.sub.i {slot} {value}"),
            Self::Add => "add".to_string(),
            Self::AddInt => "add.int".to_string(),
            Self::Sub => "sub".to_string(),
            Self::SubInt => "sub.int".to_string(),
            Self::Mul => "mul".to_string(),
            Self::MulInt => "mul.int".to_string(),
            Self::Div => "div".to_string(),
            Self::DivInt => "div.int".to_string(),
            Self::Neg => "neg".to_string(),
            Self::Compare(op) => format!("cmp.{}", op.name().to_ascii_lowercase()),
            Self::CompareInt(op) => format!("cmp.int.{}", op.name().to_ascii_lowercase()),
            Self::Jump(target) => format!("jmp {target}"),
            Self::JumpHot(target) => format!("jmp.hot {target}"),
            Self::JumpIfZero(target) => format!("jz {target}"),
            Self::JumpIfZeroHot(target) => format!("jz.hot {target}"),
            Self::Call(function, arg_count) => format!("call f{function} argc={arg_count}"),
            Self::MakeArray(count) => format!("array {count}"),
            Self::Index => "index".to_string(),
            Self::SetIndex => "set.index".to_string(),
            Self::MakeStruct(index, field_count) => format!("struct s{index} fields={field_count}"),
            Self::GetField(field) => format!("get.field {field}"),
            Self::SetField(field) => format!("set.field {field}"),
            Self::Builtin(index, arg_count) => format!("builtin b{index} argc={arg_count}"),
            Self::Return => "return".to_string(),
            Self::Print => "print".to_string(),
            Self::Halt => "halt".to_string(),
        }
    }

    pub(crate) fn remap_targets(&mut self, original_to_compiled: &[usize]) {
        match self {
            Self::Jump(target) | Self::JumpHot(target) => {
                if let Some(mapped) = original_to_compiled.get(*target) {
                    *target = *mapped;
                }
            }
            Self::JumpIfZero(target) | Self::JumpIfZeroHot(target) => {
                if let Some(mapped) = original_to_compiled.get(*target) {
                    *target = *mapped;
                }
            }
            _ => {}
        }
    }
}

fn jit_operand(value: i64, name: &str) -> Result<usize> {
    checked_non_negative_usize(value, name)
        .map_err(|error| TinyOneError::compile(format!("JIT invalid {name}: {error}")))
}
