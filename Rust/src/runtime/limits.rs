use crate::RuntimeValue;

pub(crate) const MAX_CALL_DEPTH: usize = 16;
pub(crate) const MAX_HEAP_OBJECTS: usize = 1_000_000;
pub(crate) const MAX_HEAP_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_ARRAY_LENGTH: usize = 65_536;
pub(crate) const MAX_BUFFER_BYTES: usize = 1024 * 1024;
pub(crate) const VALUE_BYTES: usize = std::mem::size_of::<RuntimeValue>();
