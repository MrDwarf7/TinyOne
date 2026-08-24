use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, atomic::AtomicI64};

use crate::runtime::ralloc_record::RallocRecord;
use crate::runtime::ralloc_vec::RallocVec;
use crate::runtime::sync::{TinyMutex, TinyThreadHandle};
use crate::runtime::value::PointerKind;
use crate::runtime::value_codec::{self, ENCODED_VALUE_BYTES};
use crate::tiny_allocator::RallocBytes;
use crate::{
    HeapRef, MAX_ARRAY_LENGTH, MAX_HEAP_BYTES, MAX_HEAP_OBJECTS, Result, TinyOneError, TypeKind,
    VALUE_BYTES, Value,
};

/// Canonical, hashable map-key representation. It mirrors `map_key_equal`:
/// integer widths compare by value, strings compare by content, pointers omit
/// cast metadata, and other heap objects compare by generational identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum MapKey {
    Integer(i128),
    String(String),
    HeapObject {
        address: usize,
        generation: u64,
    },
    Pointer {
        address: usize,
        kind: PointerKind,
        index: i64,
        field: String,
        generation: u64,
    },
}

/// Insertion-ordered map payload plus an O(1)-average lookup sidecar. Entries
/// remain Ralloc-owned; the index is rebuildable control metadata and is never
/// exposed through the language memory model.
#[derive(Debug)]
pub(crate) struct MapData {
    entries: RallocVec,
    pub(crate) index: HashMap<MapKey, usize>,
    pub(crate) pointer_indices: Vec<usize>,
}

impl MapData {
    fn new(entries: RallocVec) -> Self {
        // Avoid the first few host-side index rehashes while keeping empty and
        // small maps compact. The payload itself remains Ralloc-owned and
        // grows independently.
        const INITIAL_INDEX_CAPACITY: usize = 16;
        Self {
            entries,
            index: HashMap::with_capacity(INITIAL_INDEX_CAPACITY),
            pointer_indices: Vec::new(),
        }
    }

    pub(crate) fn remove_index(&mut self, removed: usize) {
        self.index.retain(|_, index| {
            if *index == removed {
                false
            } else {
                if *index > removed {
                    *index -= 1;
                }
                true
            }
        });
        self.pointer_indices.retain_mut(|index| {
            if *index == removed {
                false
            } else {
                if *index > removed {
                    *index -= 1;
                }
                true
            }
        });
    }
}

impl Deref for MapData {
    type Target = RallocVec;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl DerefMut for MapData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

// `HeapData` deliberately does not derive/implement `Clone`: `String`,
// `Buffer`, and `CharBuffer` own a `RallocBytes`, which wraps a real,
// capacity-bounded `ralloc::VmAllocation` that can only be duplicated via a
// fallible allocation — and `Clone::clone` is infallible, so a derived clone
// would have to panic on arena exhaustion. Callers that need to read out of a
// heap object without holding the heap lock across a recursive call (see
// `runtime/format.rs`, `runtime/aggregate.rs`, `runtime/stdlib.rs`) extract
// just the owned pieces they need instead of cloning the whole object.
#[derive(Debug)]
pub(crate) enum HeapData {
    String(RallocBytes),
    /// A dynamic array of `Value`s, physically a `RallocVec` of
    /// `value_codec::ENCODED_VALUE_BYTES`-wide encoded slots.
    Array(RallocVec),
    Buffer(RallocBytes),
    /// A named-field struct, physically a `RallocRecord` (fixed field count
    /// and names set once at construction; field *values* are mutable).
    Struct(RallocRecord),
    /// A single mutable `Value` slot, physically a fixed
    /// `value_codec::ENCODED_VALUE_BYTES`-wide `RallocBytes` — never
    /// resized, since a cell always holds exactly one value.
    Cell(RallocBytes),
    /// A key-value map, physically a `RallocVec` of `2 *
    /// value_codec::ENCODED_VALUE_BYTES`-wide slots (encoded key then
    /// encoded value, contiguous per entry). Iteration order is insertion
    /// order, matching the spec.
    Map(MapData),
    Mutex(Arc<TinyMutex>),
    Atomic(Arc<AtomicI64>),
    Thread(Arc<TinyThreadHandle>),

    // Text
    Char(u32),
    CharBuffer(RallocBytes),

    // Sequences
    #[allow(dead_code)]
    Vec(RallocVec),
    Record(RallocRecord),

    // Associative
    Dictionary(RallocVec),

    // Ownership
    /// A single owned `Value` slot — same physical shape as `Cell`, just
    /// with by-value (not by-reference) semantics at the language level.
    Box(RallocBytes),
    Alloc {
        #[allow(dead_code)]
        kind: TypeKind,
        data: RallocBytes,
    },

    // Callable
    Closure {
        function_id: u32,
        captures: RallocVec,
    },

    // Algebraic
    Sum {
        tag: u32,
        payload: Option<RallocBytes>,
    },
    /// Physically a `RallocRecord` — `variant()`/`tag()` are the record's
    /// header, `fields` live in its value slots.
    Enum(RallocRecord),
    TaggedUnion {
        tag: u32,
        payload: RallocBytes,
    },

    // Higher-level
    #[allow(dead_code)]
    Result {
        is_ok: bool,
        value: RallocBytes,
    },
    #[allow(dead_code)]
    Option {
        value: Option<RallocBytes>,
    },
    Dyn {
        type_id: u16,
        vtable_id: u32,
        value: RallocBytes,
    },

    // System
    FileDescriptor(i32),
}

#[derive(Debug)]
pub(crate) struct HeapObject {
    pub(crate) data: HeapData,
    pub(crate) type_name: String,
}

impl HeapObject {
    pub(crate) fn kind(&self) -> &'static str {
        match self.data {
            HeapData::String(_) => "string",
            HeapData::Array(_) => "array",
            HeapData::Buffer(_) => "buffer",
            HeapData::Struct(_) => "struct",
            HeapData::Cell(_) => "cell",
            HeapData::Map(_) => "map",
            HeapData::Mutex(_) => "mutex",
            HeapData::Atomic(_) => "atomic",
            HeapData::Thread(_) => "thread",
            HeapData::Char(_) => "char",
            HeapData::CharBuffer(_) => "char_buffer",
            HeapData::Vec(_) => "vec",
            HeapData::Record(_) => "record",
            HeapData::Dictionary(_) => "dictionary",
            HeapData::Box(_) => "box",
            HeapData::Alloc { .. } => "alloc",
            HeapData::Closure { .. } => "closure",
            HeapData::Sum { .. } => "sum",
            HeapData::Enum(_) => "enum",
            HeapData::TaggedUnion { .. } => "tagged_union",
            HeapData::Result { .. } => "result",
            HeapData::Option { .. } => "option",
            HeapData::Dyn { .. } => "dyn",
            HeapData::FileDescriptor(_) => "file_descriptor",
        }
    }

    #[allow(dead_code)]
    pub(crate) fn type_kind(&self) -> crate::TypeKind {
        use crate::TypeKind;
        match self.data {
            HeapData::String(_) => TypeKind::String,
            HeapData::Array(_) => TypeKind::Array,
            HeapData::Buffer(_) => TypeKind::Buffer,
            HeapData::Struct(_) => TypeKind::Struct,
            HeapData::Cell(_) => TypeKind::Cell,
            HeapData::Map(_) => TypeKind::Map,
            HeapData::Mutex(_) => TypeKind::Mutex,
            HeapData::Atomic(_) => TypeKind::Atomic,
            HeapData::Thread(_) => TypeKind::Thread,
            HeapData::Char(_) => TypeKind::Char,
            HeapData::CharBuffer(_) => TypeKind::CharBuffer,
            HeapData::Vec(_) => TypeKind::Vec,
            HeapData::Record(_) => TypeKind::Record,
            HeapData::Dictionary(_) => TypeKind::Dictionary,
            HeapData::Box(_) => TypeKind::Box,
            HeapData::Alloc { .. } => TypeKind::Alloc,
            HeapData::Closure { .. } => TypeKind::Closure,
            HeapData::Sum { .. } => TypeKind::Sum,
            HeapData::Enum(_) => TypeKind::Enum,
            HeapData::TaggedUnion { .. } => TypeKind::TaggedUnion,
            HeapData::Result { .. } => TypeKind::Result,
            HeapData::Option { .. } => TypeKind::Option,
            HeapData::Dyn { .. } => TypeKind::Dyn,
            HeapData::FileDescriptor(_) => TypeKind::FileDescriptor,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TinyHeapStats {
    pub live_objects: usize,
    pub live_bytes: usize,
    pub peak_objects: usize,
    pub peak_bytes: usize,
    pub total_allocations: u64,
    pub total_frees: u64,
    pub shutdown_frees: u64,
}

pub(crate) struct TinyHeap {
    pub(crate) objects: Vec<Option<HeapObject>>,
    pub(crate) free: Vec<usize>,
    pub(crate) generations: Vec<u64>,
    pub(crate) stats: TinyHeapStats,
    pub(crate) shutdown: bool,
    /// One fixed-width cell payload retained across free/reallocate churn.
    /// The cache is bounded to one 64-byte slot and remains protected by the
    /// heap mutex; logical heap accounting still follows live objects only.
    spare_cell: Option<RallocBytes>,
}

impl std::fmt::Debug for TinyHeap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TinyHeap")
            .field("objects", &self.objects)
            .field("free", &self.free)
            .field("generations", &self.generations)
            .field("stats", &self.stats)
            .field("shutdown", &self.shutdown)
            .field("spare_cell", &self.spare_cell.as_ref().map(|_| "<cached>"))
            .finish()
    }
}

impl Default for TinyHeap {
    fn default() -> Self {
        Self::new()
    }
}

fn expect_heap_ref(value: &Value) -> Result<&HeapRef> {
    match value {
        Value::Heap(reference) => Ok(reference),
        _ => Err(TinyOneError::runtime("Expected heap pointer")),
    }
}

fn checked_or<T>(opt: Option<T>, error: &'static str) -> Result<T> {
    opt.ok_or_else(|| TinyOneError::runtime(error))
}

impl TinyHeap {
    pub(crate) fn new() -> Self {
        Self {
            objects: vec![None],
            free: Vec::new(),
            generations: vec![0],
            stats: TinyHeapStats::default(),
            shutdown: false,
            spare_cell: None,
        }
    }

    pub(crate) fn alloc(&mut self, object: HeapObject) -> Result<HeapRef> {
        if self.shutdown {
            return Err(TinyOneError::runtime("Heap is already shut down"));
        }
        let bytes = heap_object_bytes(&object);
        self.ensure_can_allocate(bytes)?;
        if let Some(address) = self.free.pop() {
            let generation = {
                let generation = self.generations.get_mut(address).ok_or_else(|| {
                    TinyOneError::runtime(format!("Invalid heap free slot {address}"))
                })?;
                *generation = checked_or(generation.checked_add(1), "Heap generation exhausted")?;
                *generation
            };
            let target = self.objects.get_mut(address).ok_or_else(|| {
                TinyOneError::runtime(format!("Invalid heap free slot {address}"))
            })?;
            *target = Some(object);
            self.record_alloc(bytes)?;
            Ok(HeapRef {
                address,
                generation,
            })
        } else {
            if self.objects.len() >= MAX_HEAP_OBJECTS {
                return Err(TinyOneError::runtime(format!(
                    "Heap object limit {MAX_HEAP_OBJECTS} exceeded"
                )));
            }
            let address = self.objects.len();
            self.objects.push(Some(object));
            self.generations.push(1);
            let generation = 1u64;
            self.record_alloc(bytes)?;
            Ok(HeapRef {
                address,
                generation,
            })
        }
    }

    pub(crate) fn ensure_can_allocate(&self, bytes: usize) -> Result<()> {
        if self.stats.live_objects >= MAX_HEAP_OBJECTS {
            return Err(TinyOneError::runtime(format!(
                "Heap object limit {MAX_HEAP_OBJECTS} exceeded"
            )));
        }
        let next_bytes = checked_or(
            self.stats.live_bytes.checked_add(bytes),
            "Heap byte accounting overflow",
        )?;
        if next_bytes > MAX_HEAP_BYTES {
            return Err(TinyOneError::runtime(format!(
                "Heap byte limit {MAX_HEAP_BYTES} exceeded"
            )));
        }
        Ok(())
    }

    pub(crate) fn record_alloc(&mut self, bytes: usize) -> Result<()> {
        self.stats.live_objects = checked_or(
            self.stats.live_objects.checked_add(1),
            "Heap object accounting overflow",
        )?;
        self.stats.live_bytes = checked_or(
            self.stats.live_bytes.checked_add(bytes),
            "Heap byte accounting overflow",
        )?;
        self.stats.total_allocations = checked_or(
            self.stats.total_allocations.checked_add(1),
            "Heap allocation counter overflow",
        )?;
        self.stats.peak_objects = self.stats.peak_objects.max(self.stats.live_objects);
        self.stats.peak_bytes = self.stats.peak_bytes.max(self.stats.live_bytes);
        Ok(())
    }

    pub(crate) fn record_free(&mut self, bytes: usize) -> Result<()> {
        self.stats.live_objects = checked_or(
            self.stats.live_objects.checked_sub(1),
            "Heap object accounting underflow",
        )?;
        self.stats.live_bytes = checked_or(
            self.stats.live_bytes.checked_sub(bytes),
            "Heap byte accounting underflow",
        )?;
        self.stats.total_frees = checked_or(
            self.stats.total_frees.checked_add(1),
            "Heap free counter overflow",
        )?;
        Ok(())
    }

    pub(crate) fn grow_array(&mut self, target: &Value, value: Value) -> Result<usize> {
        let reference = expect_heap_ref(target)?;
        let object = self.get_address(reference.address, reference.generation)?;
        let HeapData::Array(values) = &object.data else {
            return Err(TinyOneError::runtime(format!(
                "push() expects an array, got {}",
                object.kind()
            )));
        };
        if values.len() >= MAX_ARRAY_LENGTH {
            return Err(TinyOneError::runtime(format!(
                "push() exceeds maximum length {MAX_ARRAY_LENGTH}"
            )));
        }
        self.ensure_can_allocate_delta(VALUE_BYTES)?;
        let encoded = value_codec::encode_value(&value)?;
        let len = {
            // The generation and variant were validated above while this
            // exclusive heap borrow remained live, so no occupant can change.
            let object = self
                .objects
                .get_mut(reference.address)
                .and_then(Option::as_mut)
                .expect("validated array slot remains live");
            let HeapData::Array(values) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "push() target stopped being an array",
                ));
            };
            values.push(&encoded)?;
            values.len()
        };
        self.record_growth(VALUE_BYTES)?;
        Ok(len)
    }

    pub(crate) fn shrink_array(&mut self, target: &Value) -> Result<Value> {
        let reference = expect_heap_ref(target)?;
        let object = self.get_address(reference.address, reference.generation)?;
        let HeapData::Array(_) = &object.data else {
            return Err(TinyOneError::runtime(format!(
                "pop() expects an array, got {}",
                object.kind()
            )));
        };
        let value = {
            // The generation and variant were validated above while this
            // exclusive heap borrow remained live, so no occupant can change.
            let object = self
                .objects
                .get_mut(reference.address)
                .and_then(Option::as_mut)
                .expect("validated array slot remains live");
            let HeapData::Array(values) = &mut object.data else {
                return Err(TinyOneError::runtime("pop() target stopped being an array"));
            };
            values
                .pop_with(|encoded| {
                    value_codec::decode_value(encoded.try_into().expect("encoded value stride"))
                })
                .ok_or_else(|| TinyOneError::runtime("pop() cannot pop from an empty array"))?
        };
        self.record_shrink(VALUE_BYTES)?;
        Ok(value)
    }

    pub(crate) fn ensure_can_allocate_delta(&self, bytes: usize) -> Result<()> {
        let next_bytes = checked_or(
            self.stats.live_bytes.checked_add(bytes),
            "Heap byte accounting overflow",
        )?;
        if next_bytes > MAX_HEAP_BYTES {
            return Err(TinyOneError::runtime(format!(
                "Heap byte limit {MAX_HEAP_BYTES} exceeded"
            )));
        }
        Ok(())
    }

    pub(crate) fn record_growth(&mut self, bytes: usize) -> Result<()> {
        self.stats.live_bytes = checked_or(
            self.stats.live_bytes.checked_add(bytes),
            "Heap byte accounting overflow",
        )?;
        self.stats.peak_bytes = self.stats.peak_bytes.max(self.stats.live_bytes);
        Ok(())
    }

    pub(crate) fn record_shrink(&mut self, bytes: usize) -> Result<()> {
        self.stats.live_bytes = checked_or(
            self.stats.live_bytes.checked_sub(bytes),
            "Heap byte accounting underflow",
        )?;
        Ok(())
    }

    fn alloc_data(&mut self, data: HeapData) -> Result<HeapRef> {
        self.alloc(HeapObject {
            data,
            type_name: String::new(),
        })
    }

    pub(crate) fn alloc_string(&mut self, text: impl Into<String>) -> Result<HeapRef> {
        let text = text.into();
        let bytes = RallocBytes::from_slice(text.as_bytes())
            .map_err(|e| TinyOneError::runtime(format!("Failed to allocate string: {e}")))?;
        self.alloc_data(HeapData::String(bytes))
    }

    pub(crate) fn alloc_array(&mut self, values: Vec<Value>) -> Result<HeapRef> {
        let vec = encode_into_ralloc_vec(ENCODED_VALUE_BYTES, &values)?;
        self.alloc_data(HeapData::Array(vec))
    }

    pub(crate) fn alloc_buffer(&mut self, size: usize) -> Result<HeapRef> {
        let bytes = RallocBytes::zeroed(size)
            .map_err(|e| TinyOneError::runtime(format!("Failed to allocate buffer: {e}")))?;
        self.alloc_data(HeapData::Buffer(bytes))
    }

    pub(crate) fn alloc_buffer_with(&mut self, data: Vec<u8>) -> Result<HeapRef> {
        let bytes = RallocBytes::from_slice(&data)
            .map_err(|e| TinyOneError::runtime(format!("Failed to allocate buffer: {e}")))?;
        self.alloc_data(HeapData::Buffer(bytes))
    }

    pub(crate) fn alloc_map(&mut self, entries: Vec<(Value, Value)>) -> Result<HeapRef> {
        let vec = encode_into_ralloc_map_vec(&entries)?;
        self.alloc_data(HeapData::Map(MapData::new(vec)))
    }

    pub(crate) fn alloc_struct(
        &mut self,
        type_name: impl Into<String>,
        fields: Vec<(String, Value)>,
    ) -> Result<HeapRef> {
        let record = RallocRecord::new(0, "", &fields)?;
        self.alloc(HeapObject {
            data: HeapData::Struct(record),
            type_name: type_name.into(),
        })
    }

    pub(crate) fn alloc_cell(&mut self, value: Value) -> Result<HeapRef> {
        let encoded = value_codec::encode_value(&value)?;
        let bytes = if let Some(mut bytes) = self.spare_cell.take() {
            bytes.as_mut_slice().copy_from_slice(&encoded);
            bytes
        } else {
            RallocBytes::from_slice(&encoded)
                .map_err(|e| TinyOneError::runtime(format!("Failed to allocate cell: {e}")))?
        };
        self.alloc_data(HeapData::Cell(bytes))
    }

    pub(crate) fn alloc_mutex(&mut self, m: Arc<TinyMutex>) -> Result<HeapRef> {
        self.alloc_data(HeapData::Mutex(m))
    }

    pub(crate) fn alloc_atomic(&mut self, init: i64) -> Result<HeapRef> {
        self.alloc_data(HeapData::Atomic(Arc::new(AtomicI64::new(init))))
    }

    pub(crate) fn alloc_thread(&mut self, h: Arc<TinyThreadHandle>) -> Result<HeapRef> {
        self.alloc_data(HeapData::Thread(h))
    }

    pub(crate) fn alloc_char(&mut self, scalar: u32) -> Result<HeapRef> {
        self.alloc_data(HeapData::Char(scalar))
    }

    pub(crate) fn alloc_char_buffer(&mut self, chars: Vec<u32>) -> Result<HeapRef> {
        let bytes = RallocBytes::from_slice(&pack_char_buffer(&chars))
            .map_err(|e| TinyOneError::runtime(format!("Failed to allocate char buffer: {e}")))?;
        self.alloc_data(HeapData::CharBuffer(bytes))
    }

    #[allow(dead_code)]
    pub(crate) fn alloc_vec(&mut self, values: Vec<Value>) -> Result<HeapRef> {
        let vec = encode_into_ralloc_vec(ENCODED_VALUE_BYTES, &values)?;
        self.alloc_data(HeapData::Vec(vec))
    }

    pub(crate) fn alloc_record(&mut self, fields: Vec<(String, Value)>) -> Result<HeapRef> {
        let record = RallocRecord::new(0, "", &fields)?;
        self.alloc_data(HeapData::Record(record))
    }

    pub(crate) fn alloc_dictionary(&mut self, entries: Vec<(Value, Value)>) -> Result<HeapRef> {
        let vec = encode_into_ralloc_map_vec(&entries)?;
        self.alloc_data(HeapData::Dictionary(vec))
    }

    pub(crate) fn alloc_box(&mut self, value: Value) -> Result<HeapRef> {
        self.alloc_data(HeapData::Box(alloc_value_slot(&value)?))
    }

    pub(crate) fn alloc_raw(&mut self, kind: TypeKind, data: Vec<u8>) -> Result<HeapRef> {
        let bytes = RallocBytes::from_slice(&data)
            .map_err(|e| TinyOneError::runtime(format!("Failed to allocate raw alloc: {e}")))?;
        self.alloc_data(HeapData::Alloc { kind, data: bytes })
    }

    pub(crate) fn alloc_closure(
        &mut self,
        function_id: u32,
        captures: Vec<Value>,
    ) -> Result<HeapRef> {
        let captures = encode_into_ralloc_vec(ENCODED_VALUE_BYTES, &captures)?;
        self.alloc_data(HeapData::Closure {
            function_id,
            captures,
        })
    }

    pub(crate) fn alloc_sum(&mut self, tag: u32, payload: Option<Value>) -> Result<HeapRef> {
        let payload = payload.as_ref().map(alloc_value_slot).transpose()?;
        self.alloc_data(HeapData::Sum { tag, payload })
    }

    pub(crate) fn alloc_enum(
        &mut self,
        type_name: impl Into<String>,
        variant: impl Into<String>,
        tag: u32,
        fields: Vec<(String, Value)>,
    ) -> Result<HeapRef> {
        let record = RallocRecord::new(tag, &variant.into(), &fields)?;
        self.alloc(HeapObject {
            data: HeapData::Enum(record),
            type_name: type_name.into(),
        })
    }

    pub(crate) fn alloc_tagged_union(&mut self, tag: u32, payload: Value) -> Result<HeapRef> {
        let payload = alloc_value_slot(&payload)?;
        self.alloc_data(HeapData::TaggedUnion { tag, payload })
    }

    #[allow(dead_code)]
    pub(crate) fn alloc_result(&mut self, is_ok: bool, value: Value) -> Result<HeapRef> {
        let value = alloc_value_slot(&value)?;
        self.alloc_data(HeapData::Result { is_ok, value })
    }

    #[allow(dead_code)]
    pub(crate) fn alloc_option(&mut self, value: Option<Value>) -> Result<HeapRef> {
        let value = value.as_ref().map(alloc_value_slot).transpose()?;
        self.alloc_data(HeapData::Option { value })
    }

    pub(crate) fn alloc_dyn(
        &mut self,
        type_id: u16,
        vtable_id: u32,
        value: Value,
    ) -> Result<HeapRef> {
        let value = alloc_value_slot(&value)?;
        self.alloc_data(HeapData::Dyn {
            type_id,
            vtable_id,
            value,
        })
    }

    pub(crate) fn alloc_file_descriptor(&mut self, fd: i32) -> Result<HeapRef> {
        self.alloc_data(HeapData::FileDescriptor(fd))
    }

    pub(crate) fn get(&self, value: &Value) -> Result<&HeapObject> {
        let reference = expect_heap_ref(value)?;
        self.get_address(reference.address, reference.generation)
    }

    pub(crate) fn get_mut(&mut self, value: &Value) -> Result<&mut HeapObject> {
        let reference = expect_heap_ref(value)?;
        self.get_address_mut(reference.address, reference.generation)
    }

    pub(crate) fn ref_at(&self, address: usize) -> Result<HeapRef> {
        Ok(HeapRef {
            address,
            generation: self.current_generation(address)?,
        })
    }

    pub(crate) fn current_generation(&self, address: usize) -> Result<u64> {
        self.current_object(address)?;
        self.generations
            .get(address)
            .copied()
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid heap pointer {address}")))
    }

    pub(crate) fn get_address(&self, address: usize, generation: u64) -> Result<&HeapObject> {
        let obj = self.current_object(address)?;
        let current_generation = self
            .generations
            .get(address)
            .copied()
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid heap pointer {address}")))?;
        if generation != 0 && current_generation != generation {
            return Err(TinyOneError::runtime(format!(
                "Stale heap pointer {address}"
            )));
        }
        Ok(obj)
    }

    pub(crate) fn get_address_mut(
        &mut self,
        address: usize,
        generation: u64,
    ) -> Result<&mut HeapObject> {
        self.current_object(address)?;
        let current_generation = self
            .generations
            .get(address)
            .copied()
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid heap pointer {address}")))?;
        if generation != 0 && current_generation != generation {
            return Err(TinyOneError::runtime(format!(
                "Stale heap pointer {address}"
            )));
        }
        self.objects
            .get_mut(address)
            .and_then(Option::as_mut)
            .ok_or_else(|| {
                TinyOneError::runtime(format!("Use after free for heap pointer {address}"))
            })
    }

    pub(crate) fn current_object(&self, address: usize) -> Result<&HeapObject> {
        if address == 0 || address >= self.objects.len() {
            return Err(TinyOneError::runtime(format!(
                "Invalid heap pointer {address}"
            )));
        }
        self.objects
            .get(address)
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                TinyOneError::runtime(format!("Use after free for heap pointer {address}"))
            })
    }

    pub(crate) fn free(&mut self, value: &Value) -> Result<()> {
        let reference = expect_heap_ref(value)?;
        self.get_address(reference.address, reference.generation)?;
        let bytes = heap_object_bytes(self.current_object(reference.address)?);
        let target = self.objects.get_mut(reference.address).ok_or_else(|| {
            TinyOneError::runtime(format!("Invalid heap pointer {}", reference.address))
        })?;
        let object = target.take().ok_or_else(|| {
            TinyOneError::runtime(format!(
                "Use after free for heap pointer {}",
                reference.address
            ))
        })?;
        if self.spare_cell.is_none()
            && let HeapData::Cell(bytes) = object.data
        {
            self.spare_cell = Some(bytes);
        }
        self.free.push(reference.address);
        self.record_free(bytes)?;
        Ok(())
    }

    pub(crate) fn stats(&self) -> TinyHeapStats {
        self.stats
    }

    pub(crate) fn shutdown(&mut self) -> TinyHeapStats {
        if self.shutdown {
            return self.stats;
        }
        let live_objects = self.stats.live_objects;
        for slot in self.objects.iter_mut().skip(1) {
            *slot = None;
        }
        self.free.clear();
        self.stats.live_objects = 0;
        self.stats.live_bytes = 0;
        self.stats.total_frees += live_objects as u64;
        self.stats.shutdown_frees += live_objects as u64;
        self.spare_cell = None;
        self.shutdown = true;
        self.stats
    }
}

// Notional heap budget charged per spawned OS thread. Actual OS stack cost is
// typically 2–8 MB, but we charge a smaller sentinel so the heap byte limit
// still acts as a thread-count guard without being unusably restrictive.
const THREAD_HEAP_WEIGHT: usize = 64 * 1024; // 64 KiB per thread

pub(crate) fn heap_object_bytes(object: &HeapObject) -> usize {
    match &object.data {
        HeapData::String(text) => text.len(),
        HeapData::Array(values) => values.len().saturating_mul(VALUE_BYTES),
        HeapData::Buffer(data) => data.len(),
        HeapData::Struct(record) => object.type_name.len() + record.byte_len(),
        HeapData::Cell(bytes) => bytes.len(),
        HeapData::Map(entries) => entries.len().saturating_mul(VALUE_BYTES * 2),
        HeapData::Mutex(_) => std::mem::size_of::<TinyMutex>() + 2 * std::mem::size_of::<usize>(),
        HeapData::Atomic(_) => std::mem::size_of::<AtomicI64>() + 2 * std::mem::size_of::<usize>(),
        HeapData::Thread(_) => THREAD_HEAP_WEIGHT,
        HeapData::Char(_) => std::mem::size_of::<u32>(),
        HeapData::CharBuffer(bytes) => bytes.len(),
        HeapData::Vec(values) => values.len() * VALUE_BYTES,
        HeapData::Record(record) => record.byte_len(),
        HeapData::Dictionary(entries) => entries.len() * VALUE_BYTES * 2,
        HeapData::Box(bytes) => bytes.len(),
        HeapData::Alloc { data, .. } => data.len(),
        HeapData::Closure { captures, .. } => captures.len() * VALUE_BYTES,
        HeapData::Sum { .. } => VALUE_BYTES * 2,
        HeapData::Enum(record) => object.type_name.len() + record.byte_len(),
        HeapData::TaggedUnion { .. } => VALUE_BYTES + std::mem::size_of::<u32>(),
        HeapData::Result { .. } => VALUE_BYTES + 1,
        HeapData::Option { value, .. } => {
            if value.is_some() {
                VALUE_BYTES
            } else {
                1
            }
        }
        HeapData::Dyn { .. } => {
            VALUE_BYTES + std::mem::size_of::<u16>() + std::mem::size_of::<u32>()
        }
        HeapData::FileDescriptor(_) => std::mem::size_of::<i32>(),
    }
}

/// Packs `chars` into little-endian bytes for storage in a `CharBuffer`'s
/// `RallocBytes`. Manual byte packing (rather than an unsafe transmute) keeps
/// `CharBuffer` free of alignment assumptions about the underlying arena
/// allocation, matching how `runtime::pointers::runtime_read_uint`/
/// `runtime_write_uint` already pack/unpack multi-byte integers by hand.
pub(crate) fn pack_char_buffer(chars: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(chars));
    for ch in chars {
        bytes.extend_from_slice(&ch.to_le_bytes());
    }
    bytes
}

/// Inverse of [`pack_char_buffer`]. `bytes.len()` is always a multiple of 4
/// since it only ever comes from `pack_char_buffer`.
pub(crate) fn unpack_char_buffer(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect()
}

/// Interprets a `HeapData::String`'s bytes as UTF-8.
///
/// Always succeeds in practice — a `String` heap object's `RallocBytes` is
/// only ever constructed from a valid Rust `String` (`alloc_string`) and
/// never mutated in place afterward — but returns a `Result` since the bytes
/// themselves are opaque to the type system.
pub(crate) fn heap_str(bytes: &RallocBytes) -> Result<&str> {
    std::str::from_utf8(bytes.as_slice())
        .map_err(|_| TinyOneError::runtime("String heap object contained invalid UTF-8"))
}

/// Allocates a single `value_codec::ENCODED_VALUE_BYTES`-wide `RallocBytes`
/// slot holding `value` — the shared shape behind `HeapData::Cell`, `Box`,
/// `Sum`/`TaggedUnion`/`Result`/`Option`/`Dyn`'s payloads.
fn alloc_value_slot(value: &Value) -> Result<RallocBytes> {
    let encoded = value_codec::encode_value(value)?;
    RallocBytes::from_slice(&encoded)
        .map_err(|e| TinyOneError::runtime(format!("Failed to allocate value: {e}")))
}

/// Builds a `RallocVec` of `stride`-wide encoded `Value` slots from `values`
/// — shared by `HeapData::Array`/`Vec`/`Closure`'s captures.
fn encode_into_ralloc_vec(stride: usize, values: &[Value]) -> Result<RallocVec> {
    let mut vec = RallocVec::with_capacity(stride, values.len())?;
    for value in values {
        vec.push(&value_codec::encode_value(value)?)?;
    }
    Ok(vec)
}

/// Builds a `RallocVec` of `2 * ENCODED_VALUE_BYTES`-wide `(key, value)`
/// slots from `entries` — shared by `HeapData::Map`/`Dictionary`.
fn encode_into_ralloc_map_vec(entries: &[(Value, Value)]) -> Result<RallocVec> {
    let mut vec = RallocVec::with_capacity(2 * ENCODED_VALUE_BYTES, entries.len())?;
    for (key, value) in entries {
        let mut pair = [0u8; 2 * ENCODED_VALUE_BYTES];
        pair[..ENCODED_VALUE_BYTES].copy_from_slice(&value_codec::encode_value(key)?);
        pair[ENCODED_VALUE_BYTES..].copy_from_slice(&value_codec::encode_value(value)?);
        vec.push(&pair)?;
    }
    Ok(vec)
}

/// Decodes a single `value_codec::ENCODED_VALUE_BYTES`-wide `RallocBytes`
/// slot back into a `Value` — the inverse of [`alloc_value_slot`].
pub(crate) fn decode_value_slot(bytes: &RallocBytes) -> Value {
    value_codec::decode_value(bytes.as_slice().try_into().unwrap())
}

/// Decodes every element of a `HeapData::Array`/`Vec`-backed `RallocVec`
/// into an owned `Vec<Value>` snapshot. Used by call sites that need to
/// recurse or iterate without holding the heap lock (e.g. `Display`
/// formatting) — replaces the whole-object `.clone()` pattern that's no
/// longer possible now that these containers own real Ralloc memory.
pub(crate) fn decode_array_values(vec: &RallocVec) -> Vec<Value> {
    (0..vec.len())
        .map(|i| {
            let bytes = vec.get(i).expect("index within bounds");
            value_codec::decode_value(bytes.try_into().unwrap())
        })
        .collect()
}

/// Decodes every `(key, value)` pair of a `HeapData::Map`/`Dictionary`-backed
/// `RallocVec` (`stride = 2 * ENCODED_VALUE_BYTES`, key then value per
/// entry) into an owned `Vec<(Value, Value)>` snapshot, in insertion order.
pub(crate) fn decode_map_entries(vec: &RallocVec) -> Vec<(Value, Value)> {
    (0..vec.len())
        .map(|i| {
            let pair = vec.get(i).expect("index within bounds");
            let key = value_codec::decode_value(pair[..ENCODED_VALUE_BYTES].try_into().unwrap());
            let value = value_codec::decode_value(pair[ENCODED_VALUE_BYTES..].try_into().unwrap());
            (key, value)
        })
        .collect()
}

pub(crate) fn encoded_map_key(vec: &RallocVec, index: usize) -> Option<&[u8; ENCODED_VALUE_BYTES]> {
    vec.get_part(index, 0, ENCODED_VALUE_BYTES)?.try_into().ok()
}

pub(crate) fn decode_map_value(vec: &RallocVec, index: usize) -> Option<Value> {
    let encoded = vec.get_part(index, ENCODED_VALUE_BYTES, ENCODED_VALUE_BYTES)?;
    Some(value_codec::decode_value(encoded.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_heap_data_variants_are_allocatable() {
        use crate::TypeKind;
        use crate::Value;

        let mut heap = TinyHeap::new();

        let r = heap.alloc_char(65u32).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "char"
        );

        let r = heap.alloc_char_buffer(vec![65u32, 66u32]).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "char_buffer"
        );

        let r = heap.alloc_vec(vec![]).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "vec"
        );

        let r = heap
            .alloc_record(vec![("x".to_string(), Value::I64(1))])
            .unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "record"
        );

        let r = heap.alloc_dictionary(vec![]).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "dictionary"
        );

        let r = heap.alloc_box(Value::I64(42)).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "box"
        );

        let r = heap.alloc_raw(TypeKind::I32, vec![0u8; 4]).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "alloc"
        );

        let r = heap.alloc_closure(0u32, vec![]).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "closure"
        );

        let r = heap.alloc_sum(0u32, None).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "sum"
        );

        let r = heap.alloc_enum("Test", "Variant", 0u32, vec![]).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "enum"
        );

        let r = heap.alloc_tagged_union(0u32, Value::Unit).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "tagged_union"
        );

        let r = heap.alloc_result(true, Value::Unit).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "result"
        );

        let r = heap.alloc_option(None).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "option"
        );

        let r = heap.alloc_dyn(0u16, 0u32, Value::Unit).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "dyn"
        );

        let r = heap.alloc_file_descriptor(1i32).unwrap();
        assert_eq!(
            heap.get_address(r.address, r.generation).unwrap().kind(),
            "file_descriptor"
        );
    }

    #[test]
    fn heap_can_alloc_mutex_atomic_thread_variants() {
        use crate::runtime::sync::TinyMutex;
        let mut heap = TinyHeap::new();

        let m = TinyMutex::new();
        let hr = heap.alloc_mutex(m).unwrap();
        let obj = heap.get_address(hr.address, hr.generation).unwrap();
        assert_eq!(obj.kind(), "mutex");

        let hr = heap.alloc_atomic(7).unwrap();
        let obj = heap.get_address(hr.address, hr.generation).unwrap();
        assert_eq!(obj.kind(), "atomic");
    }

    #[test]
    fn recycled_cell_payload_preserves_generation_and_accounting() {
        let mut heap = TinyHeap::new();
        let first = heap.alloc_cell(Value::I64(7)).unwrap();
        heap.free(&Value::Heap(first)).unwrap();
        assert_eq!(heap.stats.live_objects, 0);
        assert_eq!(heap.stats.live_bytes, 0);
        assert!(heap.spare_cell.is_some());

        let second = heap.alloc_cell(Value::I64(9)).unwrap();
        assert_eq!(second.address, first.address);
        assert!(second.generation > first.generation);
        assert!(heap.get_address(first.address, first.generation).is_err());
        let object = heap.get_address(second.address, second.generation).unwrap();
        let HeapData::Cell(bytes) = &object.data else {
            panic!("reused object should remain a cell");
        };
        assert_eq!(decode_value_slot(bytes), Value::I64(9));
        assert_eq!(heap.stats.live_objects, 1);
        assert_eq!(heap.stats.live_bytes, VALUE_BYTES);
        assert!(heap.spare_cell.is_none());

        heap.free(&Value::Heap(second)).unwrap();
        heap.shutdown();
        assert!(heap.spare_cell.is_none());
    }
}
