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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeValue {
    Int(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    Heap(HeapRef),
    Pointer(RawPointer),
}

impl Default for RuntimeValue {
    fn default() -> Self {
        Self::Int(0)
    }
}

pub(crate) type Value = RuntimeValue;
