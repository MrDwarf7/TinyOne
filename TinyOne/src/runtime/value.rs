use crate::TypeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapRef {
    pub(crate) address: usize,
    pub(crate) generation: u64,
}

/// The kind of location a [`RawPointer`] refers to. A small closed set —
/// every construction site passes one of these five literals — so this is
/// an enum rather than a `String`, unlike `field` (see below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PointerKind {
    Null,
    Object,
    Array,
    Buffer,
    Field,
}

impl PointerKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PointerKind::Null => "null",
            PointerKind::Object => "object",
            PointerKind::Array => "array",
            PointerKind::Buffer => "buffer",
            PointerKind::Field => "field",
        }
    }

    /// Stable numeric id used by `runtime::value_codec`'s fixed-width byte
    /// encoding — not part of any external wire format, safe to renumber.
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            PointerKind::Null => 0,
            PointerKind::Object => 1,
            PointerKind::Array => 2,
            PointerKind::Buffer => 3,
            PointerKind::Field => 4,
        }
    }

    pub(crate) fn from_u8(byte: u8) -> Option<PointerKind> {
        match byte {
            0 => Some(PointerKind::Null),
            1 => Some(PointerKind::Object),
            2 => Some(PointerKind::Array),
            3 => Some(PointerKind::Buffer),
            4 => Some(PointerKind::Field),
            _ => None,
        }
    }
}

/// The numeric reinterpretation a pointer has been cast to via `cast_ptr()`.
/// Also a small closed set, validated against this exact list at the one
/// place a cast name string is accepted (`runtime_cast_pointer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CastKind {
    None,
    U8,
    U16,
    U32,
    I8,
    I16,
    I32,
}

impl CastKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CastKind::None => "",
            CastKind::U8 => "u8",
            CastKind::U16 => "u16",
            CastKind::U32 => "u32",
            CastKind::I8 => "i8",
            CastKind::I16 => "i16",
            CastKind::I32 => "i32",
        }
    }

    /// Parses a user-supplied cast type name, e.g. from `cast_ptr(p, "i32")`.
    pub(crate) fn parse(name: &str) -> Option<CastKind> {
        match name {
            "u8" => Some(CastKind::U8),
            "u16" => Some(CastKind::U16),
            "u32" => Some(CastKind::U32),
            "i8" => Some(CastKind::I8),
            "i16" => Some(CastKind::I16),
            "i32" => Some(CastKind::I32),
            _ => None,
        }
    }

    /// Stable numeric id used by `runtime::value_codec`'s fixed-width byte
    /// encoding — not part of any external wire format, safe to renumber.
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            CastKind::None => 0,
            CastKind::U8 => 1,
            CastKind::U16 => 2,
            CastKind::U32 => 3,
            CastKind::I8 => 4,
            CastKind::I16 => 5,
            CastKind::I32 => 6,
        }
    }

    pub(crate) fn from_u8(byte: u8) -> Option<CastKind> {
        match byte {
            0 => Some(CastKind::None),
            1 => Some(CastKind::U8),
            2 => Some(CastKind::U16),
            3 => Some(CastKind::U32),
            4 => Some(CastKind::I8),
            5 => Some(CastKind::I16),
            6 => Some(CastKind::I32),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPointer {
    pub(crate) address: usize,
    pub(crate) kind: PointerKind,
    pub(crate) index: i64,
    pub(crate) field: String,
    pub(crate) generation: u64,
    pub(crate) cast: CastKind,
}

impl RawPointer {
    pub(crate) fn new(
        address: usize,
        kind: PointerKind,
        index: i64,
        field: impl Into<String>,
        generation: u64,
        cast: CastKind,
    ) -> Self {
        Self {
            address,
            kind,
            index,
            field: field.into(),
            generation,
            cast,
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
        Self::I64(0)
    }
}

pub(crate) type Value = RuntimeValue;
