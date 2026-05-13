use crate::{
    HeapRef, MAX_ARRAY_LENGTH, MAX_HEAP_BYTES, MAX_HEAP_OBJECTS, Result, TinyOneError, VALUE_BYTES,
    Value,
};

#[derive(Debug, Clone)]
pub(crate) enum HeapData {
    String(String),
    Array(Vec<Value>),
    Buffer(Vec<u8>),
    Struct(Vec<(String, Value)>),
    Cell(Value),
    Map(Vec<(Value, Value)>),
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Default)]
pub(crate) struct TinyHeap {
    pub(crate) objects: Vec<Option<HeapObject>>,
    pub(crate) free: Vec<usize>,
    pub(crate) generations: Vec<u64>,
    pub(crate) stats: TinyHeapStats,
    pub(crate) shutdown: bool,
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
        }
    }

    pub(crate) fn alloc(&mut self, object: HeapObject) -> Result<HeapRef> {
        if self.shutdown {
            return Err(TinyOneError::runtime("Heap is already shut down"));
        }
        let bytes = heap_object_bytes(&object);
        self.ensure_can_allocate(bytes)?;
        if let Some(address) = self.free.pop() {
            self.generations[address] = checked_or(
                self.generations[address].checked_add(1),
                "Heap generation exhausted",
            )?;
            self.objects[address] = Some(object);
            self.record_alloc(bytes)?;
            Ok(HeapRef {
                address,
                generation: self.generations[address],
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
            self.record_alloc(bytes)?;
            Ok(HeapRef {
                address,
                generation: 1,
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
        self.get_address(reference.address, reference.generation)?;
        let object = self.objects[reference.address]
            .as_ref()
            .expect("current object");
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
        let len = {
            let object = self.get_address_mut(reference.address, reference.generation)?;
            let HeapData::Array(values) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "push() target stopped being an array",
                ));
            };
            values.push(value);
            values.len()
        };
        self.record_growth(VALUE_BYTES)?;
        Ok(len)
    }

    pub(crate) fn shrink_array(&mut self, target: &Value) -> Result<Value> {
        let reference = expect_heap_ref(target)?;
        self.get_address(reference.address, reference.generation)?;
        let object = self.objects[reference.address]
            .as_ref()
            .expect("current object");
        let HeapData::Array(_) = &object.data else {
            return Err(TinyOneError::runtime(format!(
                "pop() expects an array, got {}",
                object.kind()
            )));
        };
        let value = {
            let object = self.get_address_mut(reference.address, reference.generation)?;
            let HeapData::Array(values) = &mut object.data else {
                return Err(TinyOneError::runtime("pop() target stopped being an array"));
            };
            values
                .pop()
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
        self.alloc_data(HeapData::String(text.into()))
    }

    pub(crate) fn alloc_array(&mut self, values: Vec<Value>) -> Result<HeapRef> {
        self.alloc_data(HeapData::Array(values))
    }

    pub(crate) fn alloc_buffer(&mut self, size: usize) -> Result<HeapRef> {
        self.alloc_data(HeapData::Buffer(vec![0; size]))
    }

    pub(crate) fn alloc_buffer_with(&mut self, data: Vec<u8>) -> Result<HeapRef> {
        self.alloc_data(HeapData::Buffer(data))
    }

    pub(crate) fn alloc_map(&mut self, entries: Vec<(Value, Value)>) -> Result<HeapRef> {
        self.alloc_data(HeapData::Map(entries))
    }

    pub(crate) fn alloc_struct(
        &mut self,
        type_name: impl Into<String>,
        fields: Vec<(String, Value)>,
    ) -> Result<HeapRef> {
        self.alloc(HeapObject {
            data: HeapData::Struct(fields),
            type_name: type_name.into(),
        })
    }

    pub(crate) fn alloc_cell(&mut self, value: Value) -> Result<HeapRef> {
        self.alloc_data(HeapData::Cell(value))
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
        Ok(self.generations[address])
    }

    pub(crate) fn get_address(&self, address: usize, generation: u64) -> Result<&HeapObject> {
        let obj = self.current_object(address)?;
        if generation != 0 && self.generations[address] != generation {
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
        if generation != 0 && self.generations[address] != generation {
            return Err(TinyOneError::runtime(format!(
                "Stale heap pointer {address}"
            )));
        }
        Ok(self.objects[address].as_mut().expect("current object"))
    }

    pub(crate) fn current_object(&self, address: usize) -> Result<&HeapObject> {
        if address == 0 || address >= self.objects.len() {
            return Err(TinyOneError::runtime(format!(
                "Invalid heap pointer {address}"
            )));
        }
        self.objects[address].as_ref().ok_or_else(|| {
            TinyOneError::runtime(format!("Use after free for heap pointer {address}"))
        })
    }

    pub(crate) fn free(&mut self, value: &Value) -> Result<()> {
        let reference = expect_heap_ref(value)?;
        self.get_address(reference.address, reference.generation)?;
        let bytes = heap_object_bytes(
            self.objects[reference.address]
                .as_ref()
                .expect("current object"),
        );
        self.objects[reference.address] = None;
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
        self.shutdown = true;
        self.stats
    }
}

pub(crate) fn heap_object_bytes(object: &HeapObject) -> usize {
    match &object.data {
        HeapData::String(text) => text.len(),
        HeapData::Array(values) => values.len().saturating_mul(VALUE_BYTES),
        HeapData::Buffer(data) => data.len(),
        HeapData::Struct(fields) => {
            object.type_name.len()
                + fields
                    .iter()
                    .map(|(name, _)| name.len() + VALUE_BYTES)
                    .sum::<usize>()
        }
        HeapData::Cell(_) => VALUE_BYTES,
        HeapData::Map(entries) => entries.len().saturating_mul(VALUE_BYTES * 2),
    }
}
