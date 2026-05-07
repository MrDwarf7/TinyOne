use crate::{Result, TinyOneError, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub fn snapshot(&self) -> Vec<Value> {
        self.values.clone()
    }
}
