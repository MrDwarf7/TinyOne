pub(crate) mod aggregate;
pub(crate) mod arithmetic;
pub(crate) mod builtins;
pub(crate) mod context;
pub(crate) mod format;
pub(crate) mod heap;
pub(crate) mod limits;
pub(crate) mod memory;
pub(crate) mod pointers;
pub(crate) mod stdlib;
pub(crate) mod sync;
pub(crate) mod typing;
pub(crate) mod value;
pub(crate) mod vm;

pub(crate) use aggregate::{
    expect_string, runtime_array_pop, runtime_array_push, runtime_get_field, runtime_index,
    runtime_make_array, runtime_make_struct, runtime_set_field, runtime_set_index,
};
pub(crate) use arithmetic::{
    checked_bounded_len, checked_byte_range, checked_collection_index, checked_div,
    checked_div_int, checked_non_negative_usize, checked_payload_bytes, expect_int, floor_div,
    integer_value_from_kind, pop_args, runtime_add, runtime_add_int, runtime_cast_int,
    runtime_compare, runtime_compare_int, runtime_integer_kind, runtime_integer_value,
    runtime_is_false, runtime_mul, runtime_mul_int, runtime_neg, runtime_null, runtime_sub,
    runtime_sub_int,
};
pub(crate) use builtins::runtime_call_builtin;
pub(crate) use context::TinyRuntimeContext;
pub(crate) use format::runtime_print;
pub use heap::TinyHeapStats;
pub(crate) use heap::{HeapData, TinyHeap};
pub(crate) use limits::{
    MAX_ARRAY_LENGTH, MAX_BUFFER_BYTES, MAX_CALL_DEPTH, MAX_HEAP_BYTES, MAX_HEAP_OBJECTS,
    VALUE_BYTES,
};
pub use memory::TinyMemory;
pub(crate) use pointers::{
    expect_pointer, runtime_cast_pointer, runtime_make_buffer, runtime_make_field_pointer,
    runtime_make_pointer, runtime_pointer_add, runtime_pointer_address, runtime_pointer_at,
    runtime_pointer_base, runtime_pointer_eq, runtime_pointer_field, runtime_pointer_kind,
    runtime_pointer_load, runtime_pointer_offset, runtime_pointer_store, runtime_pointer_type,
    runtime_read_uint, runtime_write_uint, validate_pointer_base,
};
pub use typing::TypeKind;
pub(crate) use value::Value;
pub use value::{HeapRef, RawPointer, RuntimeValue};
pub use vm::{TinyRunReport, VM};
pub(crate) use sync::{TinyMutex, TinyThreadHandle};
