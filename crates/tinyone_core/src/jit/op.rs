use crate::{BUILTINS, Instr, Op, Result, TinyOneError, checked_non_negative_usize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedSlotCompareJump {
    // Verification caps both a chunk and its slot table at 65,536 entries.
    // Packing their zero-based indexes into u16s keeps every JitOp at 24 bytes.
    value:    i64,
    metadata: u64,
}

/// Three verifier-bounded slot indexes packed into 48 bits. Keeping this
/// compact preserves the cold `JitOp` size while allowing a map get/add/store
/// sequence to avoid transient operand-stack values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedMapGetAdd {
    metadata: u64,
}

impl PackedMapGetAdd {
    pub(crate) fn new(total: usize, map: usize, key: usize) -> Option<Self> {
        let total = u16::try_from(total).ok()?;
        let map = u16::try_from(map).ok()?;
        let key = u16::try_from(key).ok()?;
        Some(Self {
            metadata: u64::from(total) | (u64::from(map) << 16) | (u64::from(key) << 32),
        })
    }

    pub(crate) fn total_slot(self) -> usize {
        self.metadata as u16 as usize
    }

    pub(crate) fn map_slot(self) -> usize {
        (self.metadata >> 16) as u16 as usize
    }

    pub(crate) fn key_slot(self) -> usize {
        (self.metadata >> 32) as u16 as usize
    }
}

/// Four verifier-bounded slot indexes plus an immediate multiplier for the
/// counted-loop `map_set(map, key, value * K)` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedMapSetMul {
    multiplier: i64,
    metadata:   u64,
}

impl PackedMapSetMul {
    pub(crate) fn new(map: usize, key: usize, value: usize, destination: usize, multiplier: i64) -> Option<Self> {
        let map = u16::try_from(map).ok()?;
        let key = u16::try_from(key).ok()?;
        let value = u16::try_from(value).ok()?;
        let destination = u16::try_from(destination).ok()?;
        Some(Self {
            multiplier,
            metadata: u64::from(map)
                | (u64::from(key) << 16)
                | (u64::from(value) << 32)
                | (u64::from(destination) << 48),
        })
    }

    pub(crate) fn map_slot(self) -> usize {
        self.metadata as u16 as usize
    }

    pub(crate) fn key_slot(self) -> usize {
        (self.metadata >> 16) as u16 as usize
    }

    pub(crate) fn value_slot(self) -> usize {
        (self.metadata >> 32) as u16 as usize
    }

    pub(crate) fn destination_slot(self) -> usize {
        (self.metadata >> 48) as u16 as usize
    }

    pub(crate) fn multiplier(self) -> i64 {
        self.multiplier
    }
}

/// Verified builtin calls that occur in collection and cell hot paths. They
/// retain the language-level builtin semantics, but let the JIT move owned
/// operands directly instead of constructing a slice for the generic
/// string-dispatch bridge and cloning its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JitBuiltin {
    Len,
    ArrayPush,
    ArrayPop,
    VecNew,
    VecClear,
    MapNew,
    MapSet,
    MapGet,
    MapHas,
    MapDel,
    Alloc,
    Load,
    Store,
    Free,
}

impl JitBuiltin {
    fn from_builtin(index: usize, arg_count: usize) -> Option<Self> {
        match (BUILTINS.get(index)?.name, arg_count) {
            ("len", 1) => Some(Self::Len),
            ("push", 2) => Some(Self::ArrayPush),
            ("pop", 1) => Some(Self::ArrayPop),
            ("vec_new", 0) => Some(Self::VecNew),
            ("vec_clear", 1) => Some(Self::VecClear),
            ("map_new", 0) => Some(Self::MapNew),
            ("map_set", 3) => Some(Self::MapSet),
            ("map_get", 2) => Some(Self::MapGet),
            ("map_has", 2) => Some(Self::MapHas),
            ("map_del", 2) => Some(Self::MapDel),
            ("alloc", 1) => Some(Self::Alloc),
            ("load", 1) => Some(Self::Load),
            ("store", 2) => Some(Self::Store),
            ("free", 1) => Some(Self::Free),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Len => "len",
            Self::ArrayPush => "push",
            Self::ArrayPop => "pop",
            Self::VecNew => "vec.new",
            Self::VecClear => "vec.clear",
            Self::MapNew => "map.new",
            Self::MapSet => "map.set",
            Self::MapGet => "map.get",
            Self::MapHas => "map.has",
            Self::MapDel => "map.del",
            Self::Alloc => "alloc",
            Self::Load => "load",
            Self::Store => "store",
            Self::Free => "free",
        }
    }

    pub(crate) fn arg_count(self) -> usize {
        match self {
            Self::VecNew | Self::MapNew => 0,
            Self::Len | Self::ArrayPop | Self::VecClear | Self::Alloc | Self::Load | Self::Free => 1,
            Self::ArrayPush | Self::MapGet | Self::MapHas | Self::MapDel | Self::Store => 2,
            Self::MapSet => 3,
        }
    }
}

impl PackedSlotCompareJump {
    fn new(slot: usize, value: i64, op: Op, target: usize) -> Option<Self> {
        let slot = u16::try_from(slot).ok()?;
        let target = u16::try_from(target).ok()?;
        let comparison = match op {
            Op::Lt => 0,
            Op::Lte => 1,
            Op::Gt => 2,
            Op::Gte => 3,
            Op::Eq => 4,
            Op::Ne => 5,
            _ => return None,
        };
        Some(Self {
            value,
            metadata: u64::from(slot) | (u64::from(target) << 16) | (comparison << 32),
        })
    }

    pub(crate) fn slot(self) -> usize {
        (self.metadata as u16) as usize
    }

    pub(crate) fn value(self) -> i64 {
        self.value
    }

    pub(crate) fn comparison(self) -> Op {
        match (self.metadata >> 32) & 0xff {
            0 => Op::Lt,
            1 => Op::Lte,
            2 => Op::Gt,
            3 => Op::Gte,
            4 => Op::Eq,
            5 => Op::Ne,
            other => unreachable!("invalid packed comparison {other}"),
        }
    }

    pub(crate) fn target(self) -> usize {
        ((self.metadata >> 16) as u16) as usize
    }

    fn set_target(&mut self, target: usize) {
        let target = u16::try_from(target).expect("verified JIT target fits in u16");
        self.metadata = (self.metadata & !(u64::from(u16::MAX) << 16)) | (u64::from(target) << 16);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JitOp {
    PushInt(i64),
    PushNull,
    Pop,
    PushString(usize),
    Load(usize),
    LoadGlobal(usize),
    Store(usize),
    MoveSlot(usize, usize),
    StoreInt(usize, i64),
    AddSlotInt(usize, i64),
    SubSlotInt(usize, i64),
    MulSlotInt(usize, i64),
    MulSlotIntHot(usize, i64),
    DivSlotInt(usize, i64),
    DivSlotIntHot(usize, i64),
    CompareSlotIntJumpIfZero(PackedSlotCompareJump),
    CompareSlotIntJumpIfZeroHot(PackedSlotCompareJump),
    JumpIfZeroSlot(usize, usize),
    JumpIfZeroSlotHot(usize, usize),
    ArrayLenSlotJumpIfZero(usize, usize),
    MapGetAddSlots(PackedMapGetAdd),
    MapSetMulSlots(PackedMapSetMul),
    PushMulSlotInt(usize, i64),
    PushMulSlotIntHot(usize, i64),
    PushDivSlotInt(usize, i64),
    PushDivSlotIntHot(usize, i64),
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
    MakeEnum(usize, usize),
    PushBool(bool),
    /// IEEE-754 bit pattern of an `f64` literal (not the `f64` itself — `f64`
    /// is not `Eq`, and `JitOp` derives it). Reconstructed with
    /// `f64::from_bits` at the point of use.
    PushFloat(u64),
    PushFunction(usize),
    CallValue(usize),
    BuiltinDirect(JitBuiltin),
    Builtin(usize, usize),
    Return,
    Print,
    Halt,
}

impl JitOp {
    pub(crate) fn compare_slot_int_jump_if_zero(slot: usize, value: i64, op: Op, target: usize) -> Option<Self> {
        PackedSlotCompareJump::new(slot, value, op, target).map(Self::CompareSlotIntJumpIfZero)
    }

    pub(crate) fn map_get_add_slots(total: usize, map: usize, key: usize) -> Option<Self> {
        PackedMapGetAdd::new(total, map, key).map(Self::MapGetAddSlots)
    }

    pub(crate) fn map_set_mul_slots(
        map: usize,
        key: usize,
        value: usize,
        destination: usize,
        multiplier: i64,
    ) -> Option<Self> {
        PackedMapSetMul::new(map, key, value, destination, multiplier).map(Self::MapSetMulSlots)
    }

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
            Op::Call => {
                Self::Call(jit_operand(instr.arg, "function index")?, jit_operand(instr.arg2, "function arity")?)
            }
            Op::MakeArray => Self::MakeArray(jit_operand(instr.arg, "array arity")?),
            Op::Index => Self::Index,
            Op::SetIndex => Self::SetIndex,
            Op::MakeStruct => {
                Self::MakeStruct(jit_operand(instr.arg, "struct index")?, jit_operand(instr.arg2, "struct arity")?)
            }
            Op::GetField => Self::GetField(jit_operand(instr.arg, "field index")?),
            Op::SetField => Self::SetField(jit_operand(instr.arg, "field index")?),
            Op::MakeEnum => {
                Self::MakeEnum(
                    jit_operand(instr.arg, "enum variant index")?,
                    jit_operand(instr.arg2, "enum variant arity")?,
                )
            }
            Op::PushBool => Self::PushBool(instr.arg != 0),
            Op::PushFloat => Self::PushFloat(instr.arg as u64),
            Op::PushFunction => Self::PushFunction(jit_operand(instr.arg, "function index")?),
            Op::CallValue => Self::CallValue(jit_operand(instr.arg, "call arity")?),
            Op::Builtin => {
                let index = jit_operand(instr.arg, "builtin index")?;
                let arg_count = jit_operand(instr.arg2, "builtin arity")?;
                JitBuiltin::from_builtin(index, arg_count)
                    .map(Self::BuiltinDirect)
                    .unwrap_or(Self::Builtin(index, arg_count))
            }
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
            Self::CompareSlotIntJumpIfZero(operands) => Self::CompareSlotIntJumpIfZeroHot(operands),
            Self::JumpIfZeroSlot(slot, target) => Self::JumpIfZeroSlotHot(slot, target),
            Self::MulSlotInt(slot, value) => Self::MulSlotIntHot(slot, value),
            Self::DivSlotInt(slot, value) => Self::DivSlotIntHot(slot, value),
            Self::PushMulSlotInt(slot, value) => Self::PushMulSlotIntHot(slot, value),
            Self::PushDivSlotInt(slot, value) => Self::PushDivSlotIntHot(slot, value),
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
            Self::MoveSlot(source, destination) => format!("slot.move {source} {destination}"),
            Self::StoreInt(slot, value) => format!("store.i {slot} {value}"),
            Self::AddSlotInt(slot, value) => format!("slot.add.i {slot} {value}"),
            Self::SubSlotInt(slot, value) => format!("slot.sub.i {slot} {value}"),
            Self::MulSlotInt(slot, value) => format!("slot.mul.i {slot} {value}"),
            Self::MulSlotIntHot(slot, value) => format!("slot.mul.i.hot {slot} {value}"),
            Self::DivSlotInt(slot, value) => format!("slot.div.i {slot} {value}"),
            Self::DivSlotIntHot(slot, value) => format!("slot.div.i.hot {slot} {value}"),
            Self::CompareSlotIntJumpIfZero(operands) => {
                format!(
                    "slot.cmp.{}.i.jz {} {} {}",
                    operands.comparison().name().to_ascii_lowercase(),
                    operands.slot(),
                    operands.value(),
                    operands.target()
                )
            }
            Self::CompareSlotIntJumpIfZeroHot(operands) => {
                format!(
                    "slot.cmp.{}.i.jz.hot {} {} {}",
                    operands.comparison().name().to_ascii_lowercase(),
                    operands.slot(),
                    operands.value(),
                    operands.target()
                )
            }
            Self::JumpIfZeroSlot(slot, target) => format!("slot.jz {slot} {target}"),
            Self::JumpIfZeroSlotHot(slot, target) => format!("slot.jz.hot {slot} {target}"),
            Self::ArrayLenSlotJumpIfZero(slot, target) => {
                format!("array.len.slot.jz {slot} {target}")
            }
            Self::MapGetAddSlots(slots) => {
                format!("map.get.add.slots {} {} {}", slots.total_slot(), slots.map_slot(), slots.key_slot())
            }
            Self::MapSetMulSlots(slots) => {
                format!(
                    "map.set.mul.slots {} {} {} {} {}",
                    slots.map_slot(),
                    slots.key_slot(),
                    slots.value_slot(),
                    slots.destination_slot(),
                    slots.multiplier()
                )
            }
            Self::PushMulSlotInt(slot, value) => format!("push.slot.mul.i {slot} {value}"),
            Self::PushMulSlotIntHot(slot, value) => {
                format!("push.slot.mul.i.hot {slot} {value}")
            }
            Self::PushDivSlotInt(slot, value) => format!("push.slot.div.i {slot} {value}"),
            Self::PushDivSlotIntHot(slot, value) => {
                format!("push.slot.div.i.hot {slot} {value}")
            }
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
            Self::MakeEnum(index, field_count) => format!("enum v{index} fields={field_count}"),
            Self::PushBool(value) => format!("push.bool {value}"),
            Self::PushFloat(bits) => format!("push.f {}", f64::from_bits(bits)),
            Self::PushFunction(index) => format!("push.fn {index}"),
            Self::CallValue(count) => format!("call.value args={count}"),
            Self::BuiltinDirect(builtin) => format!("builtin.direct.{}", builtin.name()),
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
            Self::JumpIfZeroSlot(_, target) | Self::JumpIfZeroSlotHot(_, target) => {
                if let Some(mapped) = original_to_compiled.get(*target) {
                    *target = *mapped;
                }
            }
            Self::ArrayLenSlotJumpIfZero(_, target) => {
                if let Some(mapped) = original_to_compiled.get(*target) {
                    *target = *mapped;
                }
            }
            Self::CompareSlotIntJumpIfZero(operands) | Self::CompareSlotIntJumpIfZeroHot(operands) => {
                if let Some(mapped) = original_to_compiled.get(operands.target()) {
                    operands.set_target(*mapped);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn profile_name(self) -> &'static str {
        match self {
            Self::PushInt(_) => "push.i",
            Self::PushNull => "push.null",
            Self::Pop => "pop",
            Self::PushString(_) => "push.str",
            Self::Load(_) => "load",
            Self::LoadGlobal(_) => "load.global",
            Self::Store(_) => "store",
            Self::MoveSlot(_, _) => "slot.move",
            Self::StoreInt(_, _) => "store.i",
            Self::AddSlotInt(_, _) => "slot.add.i",
            Self::SubSlotInt(_, _) => "slot.sub.i",
            Self::MulSlotInt(_, _) => "slot.mul.i",
            Self::MulSlotIntHot(_, _) => "slot.mul.i.hot",
            Self::DivSlotInt(_, _) => "slot.div.i",
            Self::DivSlotIntHot(_, _) => "slot.div.i.hot",
            Self::CompareSlotIntJumpIfZero(_) => "slot.cmp.i.jz",
            Self::CompareSlotIntJumpIfZeroHot(_) => "slot.cmp.i.jz.hot",
            Self::JumpIfZeroSlot(_, _) => "slot.jz",
            Self::JumpIfZeroSlotHot(_, _) => "slot.jz.hot",
            Self::ArrayLenSlotJumpIfZero(_, _) => "array.len.slot.jz",
            Self::MapGetAddSlots(_) => "map.get.add.slots",
            Self::MapSetMulSlots(_) => "map.set.mul.slots",
            Self::PushMulSlotInt(_, _) => "push.slot.mul.i",
            Self::PushMulSlotIntHot(_, _) => "push.slot.mul.i.hot",
            Self::PushDivSlotInt(_, _) => "push.slot.div.i",
            Self::PushDivSlotIntHot(_, _) => "push.slot.div.i.hot",
            Self::Add => "add",
            Self::AddInt => "add.int",
            Self::Sub => "sub",
            Self::SubInt => "sub.int",
            Self::Mul => "mul",
            Self::MulInt => "mul.int",
            Self::Div => "div",
            Self::DivInt => "div.int",
            Self::Neg => "neg",
            Self::Compare(_) => "cmp",
            Self::CompareInt(_) => "cmp.int",
            Self::Jump(_) => "jmp",
            Self::JumpHot(_) => "jmp.hot",
            Self::JumpIfZero(_) => "jz",
            Self::JumpIfZeroHot(_) => "jz.hot",
            Self::Call(_, _) => "call",
            Self::MakeArray(_) => "array",
            Self::Index => "index",
            Self::SetIndex => "set.index",
            Self::MakeStruct(_, _) => "struct",
            Self::GetField(_) => "get.field",
            Self::SetField(_) => "set.field",
            Self::MakeEnum(_, _) => "enum",
            Self::PushBool(_) => "push.bool",
            Self::PushFloat(_) => "push.f",
            Self::PushFunction(_) => "push.fn",
            Self::CallValue(_) => "call.value",
            Self::BuiltinDirect(builtin) => {
                match builtin {
                    JitBuiltin::Len => "builtin.direct.len",
                    JitBuiltin::ArrayPush => "builtin.direct.push",
                    JitBuiltin::ArrayPop => "builtin.direct.pop",
                    JitBuiltin::VecNew => "builtin.direct.vec.new",
                    JitBuiltin::VecClear => "builtin.direct.vec.clear",
                    JitBuiltin::MapNew => "builtin.direct.map.new",
                    JitBuiltin::MapSet => "builtin.direct.map.set",
                    JitBuiltin::MapGet => "builtin.direct.map.get",
                    JitBuiltin::MapHas => "builtin.direct.map.has",
                    JitBuiltin::MapDel => "builtin.direct.map.del",
                    JitBuiltin::Alloc => "builtin.direct.alloc",
                    JitBuiltin::Load => "builtin.direct.load",
                    JitBuiltin::Store => "builtin.direct.store",
                    JitBuiltin::Free => "builtin.direct.free",
                }
            }
            Self::Builtin(_, _) => "builtin",
            Self::Return => "return",
            Self::Print => "print",
            Self::Halt => "halt",
        }
    }

    pub(crate) fn operand_stack_traffic(self) -> (usize, usize) {
        match self {
            Self::PushInt(_)
            | Self::PushNull
            | Self::PushString(_)
            | Self::Load(_)
            | Self::LoadGlobal(_)
            | Self::PushMulSlotInt(_, _)
            | Self::PushMulSlotIntHot(_, _)
            | Self::PushDivSlotInt(_, _)
            | Self::PushDivSlotIntHot(_, _)
            | Self::PushBool(_)
            | Self::PushFloat(_)
            | Self::PushFunction(_) => (1, 0),
            Self::Pop | Self::Store(_) | Self::JumpIfZero(_) | Self::JumpIfZeroHot(_) | Self::Print | Self::Return => {
                (0, 1)
            }
            Self::Add
            | Self::AddInt
            | Self::Sub
            | Self::SubInt
            | Self::Mul
            | Self::MulInt
            | Self::Div
            | Self::DivInt
            | Self::Compare(_)
            | Self::CompareInt(_)
            | Self::Index => (1, 2),
            Self::Neg | Self::GetField(_) => (1, 1),
            Self::Call(_, args) | Self::Builtin(_, args) => (1, args),
            Self::BuiltinDirect(builtin) => (1, builtin.arg_count()),
            Self::CallValue(args) => (1, args + 1),
            Self::MakeArray(count) | Self::MakeStruct(_, count) | Self::MakeEnum(_, count) => (1, count),
            Self::SetIndex => (0, 3),
            Self::SetField(_) => (0, 2),
            Self::StoreInt(_, _)
            | Self::MoveSlot(_, _)
            | Self::AddSlotInt(_, _)
            | Self::SubSlotInt(_, _)
            | Self::MulSlotInt(_, _)
            | Self::MulSlotIntHot(_, _)
            | Self::DivSlotInt(_, _)
            | Self::DivSlotIntHot(_, _)
            | Self::CompareSlotIntJumpIfZero(_)
            | Self::CompareSlotIntJumpIfZeroHot(_)
            | Self::JumpIfZeroSlot(_, _)
            | Self::JumpIfZeroSlotHot(_, _)
            | Self::ArrayLenSlotJumpIfZero(_, _)
            | Self::MapGetAddSlots(_)
            | Self::MapSetMulSlots(_)
            | Self::Jump(_)
            | Self::JumpHot(_)
            | Self::Halt => (0, 0),
        }
    }
}

fn jit_operand(value: i64, name: &str) -> Result<usize> {
    checked_non_negative_usize(value, name)
        .map_err(|error| TinyOneError::compile(format!("JIT invalid {name}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{JitBuiltin, JitOp};
    use crate::{Instr, Op, builtin_index};

    #[test]
    fn lowered_op_footprint_remains_bounded() {
        let bytes = std::mem::size_of::<JitOp>();
        assert!(bytes <= 24, "JitOp grew to {bytes} bytes");
    }

    #[test]
    fn lowers_collection_and_cell_builtins_to_direct_ops() {
        for (name, arg_count, expected) in [
            ("len", 1, JitBuiltin::Len),
            ("push", 2, JitBuiltin::ArrayPush),
            ("pop", 1, JitBuiltin::ArrayPop),
            ("vec_new", 0, JitBuiltin::VecNew),
            ("map_set", 3, JitBuiltin::MapSet),
            ("map_get", 2, JitBuiltin::MapGet),
            ("alloc", 1, JitBuiltin::Alloc),
            ("load", 1, JitBuiltin::Load),
            ("store", 2, JitBuiltin::Store),
            ("free", 1, JitBuiltin::Free),
        ] {
            let index = builtin_index(name).expect("known builtin") as i64;
            assert_eq!(
                JitOp::from_instr(Instr::new(Op::Builtin, index, arg_count)).unwrap(),
                JitOp::BuiltinDirect(expected),
                "{name} should lower directly"
            );
        }

        let array = builtin_index("array").expect("known builtin") as i64;
        assert!(matches!(JitOp::from_instr(Instr::new(Op::Builtin, array, 2)).unwrap(), JitOp::Builtin(_, _)));
    }
}
