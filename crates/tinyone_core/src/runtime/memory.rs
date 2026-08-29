use crate::runtime::value_codec::{self, ENCODED_VALUE_BYTES};
use crate::tiny_allocator::RallocBytes;
use crate::{Op, Result, TinyOneError, Value, checked_div_int, runtime_add_int, runtime_mul_int, runtime_sub_int};

/// VM locals/globals backed by Ralloc-owned storage.
///
/// Values use the same fixed-width representation as heap containers. The
/// transient operand stack remains a Rust collection because stack values may
/// contain arbitrarily long pointer field names, which are not representable
/// in this fixed-width format.
#[derive(Debug, Default)]
pub struct TinyMemory {
    bytes:      Option<RallocBytes>,
    slot_count: usize,
}

impl TinyMemory {
    /// Allocates VM memory, panicking if the fixed-capacity Ralloc backend is
    /// exhausted. Execution paths should use [`TinyMemory::try_new`] so
    /// allocation failures remain recoverable runtime errors.
    pub fn new(slot_count: usize) -> Self {
        Self::try_new(slot_count).unwrap_or_else(|error| panic!("failed to allocate VM memory with Ralloc: {error}"))
    }

    /// Attempts to allocate VM memory without panicking on size overflow or
    /// allocator exhaustion.
    pub fn try_new(slot_count: usize) -> Result<Self> {
        let byte_len = slot_count
            .checked_mul(ENCODED_VALUE_BYTES)
            .ok_or_else(|| TinyOneError::runtime("VM memory size overflow"))?;
        let bytes = if byte_len == 0 {
            None
        } else {
            Some(
                RallocBytes::zeroed(byte_len)
                    .map_err(|error| TinyOneError::runtime(format!("Failed to allocate VM memory: {error}")))?,
            )
        };
        let mut memory = Self { bytes, slot_count };
        memory.reset();
        Ok(memory)
    }

    pub fn reset(&mut self) {
        if let Some(bytes) = &mut self.bytes {
            let encoded = value_codec::encode_value(&Value::default()).expect("default VM value must be encodable");
            for slot in 0..self.slot_count {
                let offset = slot * ENCODED_VALUE_BYTES;
                bytes.as_mut_slice()[offset..offset + ENCODED_VALUE_BYTES].copy_from_slice(&encoded);
            }
        }
    }

    pub fn load(&self, slot: usize) -> Result<Value> {
        Ok(value_codec::decode_value(self.slot_bytes(slot)?))
    }

    pub fn store(&mut self, slot: usize, value: Value) -> Result<()> {
        let range = self.slot_range(slot)?;
        let encoded = value_codec::encode_value(&value)?;
        self.bytes.as_mut().unwrap().as_mut_slice()[range].copy_from_slice(&encoded);
        Ok(())
    }

    pub(crate) fn store_int(&mut self, slot: usize, value: i64) -> Result<()> {
        *self.slot_bytes_mut(slot)? = value_codec::encode_i64(value);
        Ok(())
    }

    fn update_int_slot(&mut self, slot: usize, value: i64, op: fn(Value, Value) -> Result<Value>) -> Result<()> {
        let next = op(self.load(slot)?, Value::I64(value))?;
        self.store(slot, next)
    }

    pub(crate) fn add_int(&mut self, slot: usize, value: i64) -> Result<()> {
        if value_codec::add_i64_in_place(self.slot_bytes_mut(slot)?, value)? {
            return Ok(());
        }
        self.update_int_slot(slot, value, runtime_add_int)
    }

    pub(crate) fn sub_int(&mut self, slot: usize, value: i64) -> Result<()> {
        if value_codec::sub_i64_in_place(self.slot_bytes_mut(slot)?, value)? {
            return Ok(());
        }
        self.update_int_slot(slot, value, runtime_sub_int)
    }

    pub(crate) fn compare_int(&self, slot: usize, value: i64, op: Op) -> Result<Option<bool>> {
        value_codec::compare_i64(self.slot_bytes(slot)?, value, op)
    }

    pub(crate) fn is_int_zero(&self, slot: usize) -> Result<Option<bool>> {
        Ok(value_codec::is_i64_zero(self.slot_bytes(slot)?))
    }

    pub(crate) fn mul_int(&self, slot: usize, value: i64) -> Result<Option<i64>> {
        value_codec::mul_i64(self.slot_bytes(slot)?, value)
    }

    pub(crate) fn div_int(&self, slot: usize, value: i64) -> Result<Option<i64>> {
        value_codec::div_i64(self.slot_bytes(slot)?, value)
    }

    pub(crate) fn mul_int_assign(&mut self, slot: usize, value: i64) -> Result<()> {
        if value_codec::mul_i64_in_place(self.slot_bytes_mut(slot)?, value)? {
            return Ok(());
        }
        self.update_int_slot(slot, value, runtime_mul_int)
    }

    pub(crate) fn div_int_assign(&mut self, slot: usize, value: i64) -> Result<()> {
        if value_codec::div_i64_in_place(self.slot_bytes_mut(slot)?, value)? {
            return Ok(());
        }
        self.update_int_slot(slot, value, checked_div_int)
    }

    pub fn snapshot(&self) -> Vec<Value> {
        (0..self.slot_count)
            .map(|slot| self.load(slot).expect("VM slot must be valid"))
            .collect()
    }

    /// Attempts to clone this memory without panicking on allocator
    /// exhaustion.
    pub fn try_clone(&self) -> Result<Self> {
        let mut copy = Self::try_new(self.slot_count)?;
        if self.slot_count != 0 {
            copy.bytes
                .as_mut()
                .expect("non-empty VM memory must have backing bytes")
                .as_mut_slice()
                .copy_from_slice(
                    self.bytes
                        .as_ref()
                        .expect("non-empty VM memory must have backing bytes")
                        .as_slice(),
                );
        }
        Ok(copy)
    }

    fn slot_range(&self, slot: usize) -> Result<std::ops::Range<usize>> {
        if slot >= self.slot_count {
            return Err(TinyOneError::runtime(format!("Invalid memory slot {slot}")));
        }
        let start = slot * ENCODED_VALUE_BYTES;
        Ok(start..start + ENCODED_VALUE_BYTES)
    }

    fn slot_bytes_mut(&mut self, slot: usize) -> Result<&mut [u8; ENCODED_VALUE_BYTES]> {
        let range = self.slot_range(slot)?;
        let bytes = self.bytes.as_mut().unwrap().as_mut_slice();
        Ok((&mut bytes[range]).try_into().expect("VM slot width is fixed"))
    }

    fn slot_bytes(&self, slot: usize) -> Result<&[u8; ENCODED_VALUE_BYTES]> {
        let range = self.slot_range(slot)?;
        let bytes = self.bytes.as_ref().unwrap().as_slice();
        Ok((&bytes[range]).try_into().expect("VM slot width is fixed"))
    }
}

impl Clone for TinyMemory {
    fn clone(&self) -> Self {
        self.try_clone()
            .unwrap_or_else(|error| panic!("failed to clone VM memory with Ralloc: {error}"))
    }
}

impl PartialEq for TinyMemory {
    fn eq(&self, other: &Self) -> bool {
        self.slot_count == other.slot_count && self.snapshot() == other.snapshot()
    }
}

impl Eq for TinyMemory {}
