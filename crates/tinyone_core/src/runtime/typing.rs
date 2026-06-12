use crate::{Result, TinyOneError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Unit,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    Bf16,
    Fp16,
    Fp32,
    Fp64,
    Char,
    String,
    CharBuffer,
    Array,
    Vec,
    Buffer,
    Map,
    Dictionary,
    Struct,
    Record,
    Pointer,
    Reference,
    Box,
    Alloc,
    Function,
    Closure,
    Sum,
    Enum,
    TaggedUnion,
    Phantom,
    Zst,
    Unsafe,
    Dyn,
    Null,
    Result,
    Option,
    FileDescriptor,
    Mutex,
    Atomic,
    Thread,
}

impl TypeKind {
    pub const fn type_id(self) -> u16 {
        match self {
            TypeKind::Unit => 0,
            TypeKind::Bool => 1,
            TypeKind::I8 => 2,
            TypeKind::I16 => 3,
            TypeKind::I32 => 4,
            TypeKind::I64 => 5,
            TypeKind::U8 => 6,
            TypeKind::U16 => 7,
            TypeKind::U32 => 8,
            TypeKind::U64 => 9,
            TypeKind::Bf16 => 10,
            TypeKind::Fp16 => 11,
            TypeKind::Fp32 => 12,
            TypeKind::Fp64 => 13,
            TypeKind::Char => 14,
            TypeKind::String => 15,
            TypeKind::CharBuffer => 16,
            TypeKind::Array => 17,
            TypeKind::Vec => 18,
            TypeKind::Buffer => 19,
            TypeKind::Map => 20,
            TypeKind::Dictionary => 21,
            TypeKind::Struct => 22,
            TypeKind::Record => 23,
            TypeKind::Pointer => 24,
            TypeKind::Reference => 25,
            TypeKind::Box => 26,
            TypeKind::Alloc => 27,
            TypeKind::Function => 28,
            TypeKind::Closure => 29,
            TypeKind::Sum => 30,
            TypeKind::Enum => 31,
            TypeKind::TaggedUnion => 32,
            TypeKind::Phantom => 33,
            TypeKind::Zst => 34,
            TypeKind::Unsafe => 35,
            TypeKind::Dyn => 36,
            TypeKind::Null => 37,
            TypeKind::Result => 38,
            TypeKind::Option => 39,
            TypeKind::FileDescriptor => 40,
            TypeKind::Mutex => 41,
            TypeKind::Atomic => 42,
            TypeKind::Thread => 43,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            TypeKind::Unit => "unit",
            TypeKind::Bool => "bool",
            TypeKind::I8 => "i8",
            TypeKind::I16 => "i16",
            TypeKind::I32 => "i32",
            TypeKind::I64 => "i64",
            TypeKind::U8 => "u8",
            TypeKind::U16 => "u16",
            TypeKind::U32 => "u32",
            TypeKind::U64 => "u64",
            TypeKind::Bf16 => "bf16",
            TypeKind::Fp16 => "fp16",
            TypeKind::Fp32 => "fp32",
            TypeKind::Fp64 => "fp64",
            TypeKind::Char => "Char",
            TypeKind::String => "String",
            TypeKind::CharBuffer => "CharBuffer",
            TypeKind::Array => "Array",
            TypeKind::Vec => "Vec",
            TypeKind::Buffer => "Buffer",
            TypeKind::Map => "Map",
            TypeKind::Dictionary => "Dictionary",
            TypeKind::Struct => "Struct",
            TypeKind::Record => "Record",
            TypeKind::Pointer => "Pointer",
            TypeKind::Reference => "Reference",
            TypeKind::Box => "Box",
            TypeKind::Alloc => "Alloc",
            TypeKind::Function => "Function",
            TypeKind::Closure => "Closure",
            TypeKind::Sum => "Sum",
            TypeKind::Enum => "Enum",
            TypeKind::TaggedUnion => "TaggedUnion",
            TypeKind::Phantom => "Phantom",
            TypeKind::Zst => "Zst",
            TypeKind::Unsafe => "Unsafe",
            TypeKind::Dyn => "Dyn",
            TypeKind::Null => "Null",
            TypeKind::Result => "Result",
            TypeKind::Option => "Option",
            TypeKind::FileDescriptor => "FileDescriptor",
            TypeKind::Mutex => "Mutex",
            TypeKind::Atomic => "Atomic",
            TypeKind::Thread => "Thread",
        }
    }

    pub const fn is_integer(self) -> bool {
        matches!(
            self,
            TypeKind::I8
                | TypeKind::I16
                | TypeKind::I32
                | TypeKind::I64
                | TypeKind::U8
                | TypeKind::U16
                | TypeKind::U32
                | TypeKind::U64
        )
    }

    pub const fn is_signed(self) -> bool {
        matches!(self, TypeKind::I8 | TypeKind::I16 | TypeKind::I32 | TypeKind::I64)
    }

    pub const fn is_unsigned(self) -> bool {
        matches!(self, TypeKind::U8 | TypeKind::U16 | TypeKind::U32 | TypeKind::U64)
    }

    pub const fn int_bits(self) -> Option<u32> {
        Some(match self {
            TypeKind::I8 | TypeKind::U8 => 8,
            TypeKind::I16 | TypeKind::U16 => 16,
            TypeKind::I32 | TypeKind::U32 => 32,
            TypeKind::I64 | TypeKind::U64 => 64,
            _ => return None,
        })
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "unit" => TypeKind::Unit,
            "bool" => TypeKind::Bool,
            "i8" => TypeKind::I8,
            "i16" => TypeKind::I16,
            "i32" => TypeKind::I32,
            "i64" => TypeKind::I64,
            "u8" => TypeKind::U8,
            "u16" => TypeKind::U16,
            "u32" => TypeKind::U32,
            "u64" => TypeKind::U64,
            "bf16" => TypeKind::Bf16,
            "fp16" => TypeKind::Fp16,
            "fp32" => TypeKind::Fp32,
            "fp64" => TypeKind::Fp64,
            "Char" => TypeKind::Char,
            "String" => TypeKind::String,
            "CharBuffer" => TypeKind::CharBuffer,
            "Array" => TypeKind::Array,
            "Vec" => TypeKind::Vec,
            "Buffer" => TypeKind::Buffer,
            "Map" => TypeKind::Map,
            "Dictionary" => TypeKind::Dictionary,
            "Struct" => TypeKind::Struct,
            "Record" => TypeKind::Record,
            "Pointer" => TypeKind::Pointer,
            "Reference" => TypeKind::Reference,
            "Box" => TypeKind::Box,
            "Alloc" => TypeKind::Alloc,
            "Function" => TypeKind::Function,
            "Closure" => TypeKind::Closure,
            "Sum" => TypeKind::Sum,
            "Enum" => TypeKind::Enum,
            "TaggedUnion" => TypeKind::TaggedUnion,
            "Phantom" => TypeKind::Phantom,
            "Zst" => TypeKind::Zst,
            "Unsafe" => TypeKind::Unsafe,
            "Dyn" => TypeKind::Dyn,
            "Null" => TypeKind::Null,
            "Result" => TypeKind::Result,
            "Option" => TypeKind::Option,
            "FileDescriptor" => TypeKind::FileDescriptor,
            "Mutex" => TypeKind::Mutex,
            "Atomic" => TypeKind::Atomic,
            "Thread" => TypeKind::Thread,
            _ => return None,
        })
    }

    /// Returns the `TypeKind` for a stack-resident `RuntimeValue`.
    /// For `Heap` values, this panics — callers must resolve via `heap.get(r).type_kind()`.
    pub fn from_runtime_value(v: &crate::Value) -> Self {
        use crate::Value;
        match v {
            Value::Unit => TypeKind::Unit,
            Value::Bool(_) => TypeKind::Bool,
            Value::I8(_) => TypeKind::I8,
            Value::I16(_) => TypeKind::I16,
            Value::I32(_) => TypeKind::I32,
            Value::I64(_) => TypeKind::I64,
            Value::U8(_) => TypeKind::U8,
            Value::U16(_) => TypeKind::U16,
            Value::U32(_) => TypeKind::U32,
            Value::U64(_) => TypeKind::U64,
            Value::Bf16(_) => TypeKind::Bf16,
            Value::Float { kind, .. } => *kind,
            Value::Null => TypeKind::Null,
            Value::Function(_) => TypeKind::Function,
            Value::Pointer(_) => TypeKind::Pointer,
            Value::Reference(_) => TypeKind::Reference,
            Value::Phantom => TypeKind::Phantom,
            Value::Zst(k) => *k,
            Value::Unsafe => TypeKind::Unsafe,
            Value::Heap(r) => {
                unimplemented!(
                    "from_runtime_value(Heap): call heap.get(r).type_kind() for heap types; \
                 HeapRef alone does not carry TypeKind — {r:?}"
                )
            }
        }
    }
}

pub fn smallest_fit_unsigned(value: u64) -> TypeKind {
    if value <= u8::MAX as u64 {
        TypeKind::U8
    } else if value <= u16::MAX as u64 {
        TypeKind::U16
    } else if value <= u32::MAX as u64 {
        TypeKind::U32
    } else {
        TypeKind::U64
    }
}

pub fn smallest_fit_signed(value: i64) -> TypeKind {
    if value >= i8::MIN as i64 && value <= i8::MAX as i64 {
        TypeKind::I8
    } else if value >= i16::MIN as i64 && value <= i16::MAX as i64 {
        TypeKind::I16
    } else if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
        TypeKind::I32
    } else {
        TypeKind::I64
    }
}

pub fn smallest_fit_literal(value: i64) -> TypeKind {
    if value >= 0 {
        smallest_fit_unsigned(value as u64)
    } else {
        smallest_fit_signed(value)
    }
}

pub fn promote_integer(lhs: TypeKind, rhs: TypeKind) -> Result<TypeKind> {
    if !lhs.is_integer() || !rhs.is_integer() {
        return Err(TinyOneError::runtime("integer promotion requires integer operands"));
    }
    if lhs == rhs {
        return Ok(lhs);
    }
    let lhs_signed = lhs.is_signed();
    let rhs_signed = rhs.is_signed();
    let lhs_bits = lhs.int_bits().unwrap_or(64);
    let rhs_bits = rhs.int_bits().unwrap_or(64);
    let target_signed = lhs_signed || rhs_signed;
    // Spec rule (typing_system.md + phase_2.md examples):
    //   target_bits = min(64, max(lhs_bits, rhs_bits) * 2)
    //   target_signed = lhs_signed || rhs_signed
    // i8 + u8   = max(8,8)*2  = 16, signed   -> i16
    // u8 + u16  = max(8,16)*2 = 32, unsigned -> u32
    // i32 + i64 = max(32,64)*2 = 128 cap 64, signed -> i64
    let target_bits = (lhs_bits.max(rhs_bits).saturating_mul(2)).min(64);
    let kind = match (target_signed, target_bits) {
        (false, b) if b <= 8 => TypeKind::U8,
        (false, b) if b <= 16 => TypeKind::U16,
        (false, b) if b <= 32 => TypeKind::U32,
        (false, _) => TypeKind::U64,
        (true, b) if b <= 8 => TypeKind::I8,
        (true, b) if b <= 16 => TypeKind::I16,
        (true, b) if b <= 32 => TypeKind::I32,
        (true, _) => TypeKind::I64,
    };
    Ok(kind)
}

pub fn integer_range(kind: TypeKind) -> Option<(i128, i128)> {
    Some(match kind {
        TypeKind::I8 => (i8::MIN as i128, i8::MAX as i128),
        TypeKind::I16 => (i16::MIN as i128, i16::MAX as i128),
        TypeKind::I32 => (i32::MIN as i128, i32::MAX as i128),
        TypeKind::I64 => (i64::MIN as i128, i64::MAX as i128),
        TypeKind::U8 => (0, u8::MAX as i128),
        TypeKind::U16 => (0, u16::MAX as i128),
        TypeKind::U32 => (0, u32::MAX as i128),
        TypeKind::U64 => (0, u64::MAX as i128),
        _ => return None,
    })
}

pub fn check_integer_range(kind: TypeKind, value: i128) -> Result<i128> {
    let (lo, hi) = integer_range(kind).ok_or_else(|| {
        TinyOneError::runtime(format!("{:?} is not an integer type with a defined range", kind.name()))
    })?;
    if value < lo || value > hi {
        return Err(TinyOneError::runtime(format!(
            "Runtime.Memory_Overflow: {} out of range for {}",
            value,
            kind.name()
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_ids_are_stable_and_unique() {
        let all = [
            TypeKind::Unit,
            TypeKind::Bool,
            TypeKind::I8,
            TypeKind::I16,
            TypeKind::I32,
            TypeKind::I64,
            TypeKind::U8,
            TypeKind::U16,
            TypeKind::U32,
            TypeKind::U64,
            TypeKind::Bf16,
            TypeKind::Fp16,
            TypeKind::Fp32,
            TypeKind::Fp64,
            TypeKind::Char,
            TypeKind::String,
            TypeKind::CharBuffer,
            TypeKind::Array,
            TypeKind::Vec,
            TypeKind::Buffer,
            TypeKind::Map,
            TypeKind::Dictionary,
            TypeKind::Struct,
            TypeKind::Record,
            TypeKind::Pointer,
            TypeKind::Reference,
            TypeKind::Box,
            TypeKind::Alloc,
            TypeKind::Function,
            TypeKind::Closure,
            TypeKind::Sum,
            TypeKind::Enum,
            TypeKind::TaggedUnion,
            TypeKind::Phantom,
            TypeKind::Zst,
            TypeKind::Unsafe,
            TypeKind::Dyn,
            TypeKind::Null,
            TypeKind::Result,
            TypeKind::Option,
            TypeKind::FileDescriptor,
            TypeKind::Mutex,
            TypeKind::Atomic,
            TypeKind::Thread,
        ];
        let ids: Vec<u16> = all.iter().map(|kind| kind.type_id()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "type ids must be unique");
        for kind in all {
            assert_eq!(TypeKind::from_name(kind.name()), Some(kind), "name round-trip for {}", kind.name());
        }
    }

    #[test]
    fn smallest_fit_inference_prefers_unsigned_for_non_negative() {
        assert_eq!(smallest_fit_literal(0), TypeKind::U8);
        assert_eq!(smallest_fit_literal(16), TypeKind::U8);
        assert_eq!(smallest_fit_literal(255), TypeKind::U8);
        assert_eq!(smallest_fit_literal(256), TypeKind::U16);
        assert_eq!(smallest_fit_literal(65_536), TypeKind::U32);
        assert_eq!(smallest_fit_literal(-1), TypeKind::I8);
        assert_eq!(smallest_fit_literal(-129), TypeKind::I16);
    }

    #[test]
    fn integer_promotion_follows_spec_examples() {
        // typing_system.md examples
        assert_eq!(promote_integer(TypeKind::I8, TypeKind::U8).unwrap(), TypeKind::I16);
        assert_eq!(promote_integer(TypeKind::U8, TypeKind::U16).unwrap(), TypeKind::U32);
        assert_eq!(promote_integer(TypeKind::I32, TypeKind::I64).unwrap(), TypeKind::I64);
    }

    #[test]
    fn range_check_reports_memory_overflow() {
        assert!(check_integer_range(TypeKind::U8, 256).is_err());
        assert_eq!(check_integer_range(TypeKind::U8, 255).unwrap(), 255i128);
        assert!(check_integer_range(TypeKind::I8, 128).is_err());
        assert_eq!(check_integer_range(TypeKind::I8, -128).unwrap(), -128i128);
    }
}
