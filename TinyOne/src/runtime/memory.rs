use crate::{Result, TinyOneError, Value, runtime_add, runtime_sub};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TinyMemory {
    values: Vec<Value>,
}

impl TinyMemory {
    pub fn new(slot_count: usize) -> Self {
        Self {
            values: vec![Value::default(); slot_count],
        }
    }

    pub fn reset(&mut self) {
        self.values.fill(Value::default());
    }

    pub fn load(&self, slot: usize) -> Result<Value> {
        self.values
            .get(slot)
            .cloned()
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid memory slot {slot}")))
    }

    pub fn store(&mut self, slot: usize, value: Value) -> Result<()> {
        let target = self
            .values
            .get_mut(slot)
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid memory slot {slot}")))?;
        *target = value;
        Ok(())
    }

    pub(crate) fn store_int(&mut self, slot: usize, value: i64) -> Result<()> {
        self.store(slot, Value::I64(value))
    }

    fn update_int_slot(
        &mut self,
        slot: usize,
        value: i64,
        op: fn(Value, Value) -> Result<Value>,
    ) -> Result<()> {
        let target = self
            .values
            .get_mut(slot)
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid memory slot {slot}")))?;
        let next = op(target.clone(), Value::I64(value))?;
        *target = next;
        Ok(())
    }

    pub(crate) fn add_int(&mut self, slot: usize, value: i64) -> Result<()> {
        self.update_int_slot(slot, value, runtime_add)
    }

    pub(crate) fn sub_int(&mut self, slot: usize, value: i64) -> Result<()> {
        self.update_int_slot(slot, value, runtime_sub)
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.values.clone()
    }
}
