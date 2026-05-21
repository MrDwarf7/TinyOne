use crate::TypeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapRef {
    pub(crate) address: usize,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPointer {
    pub(crate) address: usize,
    pub(crate) kind: String,
    pub(crate) index: i64,
    pub(crate) field: String,
    pub(crate) generation: u64,
    pub(crate) cast: String,
}

impl RawPointer {
    pub(crate) fn new(
        address: usize,
        kind: impl Into<String>,
        index: i64,
        field: impl Into<String>,
        generation: u64,
        cast: impl Into<String>,
    ) -> Self {
        Self {
            address,
            kind: kind.into(),
            index,
            field: field.into(),
            generation,
            cast: cast.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeValue {
    // Integers
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),

    // Floats
    Bf16(u16),
    Float { kind: TypeKind, bits: f64 },

    // Scalar
    Bool(bool),
    Unit,
    Null,

    // Callable
    Function(u32),

    // Reference
    Reference(RawPointer),

    // Metadata-only
    Phantom,
    Zst(TypeKind),
    Unsafe,

    // Heap-allocated types
    Heap(HeapRef),

    // Raw pointer
    Pointer(RawPointer),
}

impl Default for RuntimeValue {
    fn default() -> Self {
        Self::Unit
    }
}

pub(crate) type Value = RuntimeValue;
