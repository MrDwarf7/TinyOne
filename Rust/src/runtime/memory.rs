use crate::{Result, TinyOneError, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        self.store(slot, Value::Int(value))
    }

    fn update_int_slot(
        &mut self,
        slot: usize,
        op_name: &str,
        op: impl FnOnce(i64) -> Option<i64>,
    ) -> Result<()> {
        let target = self
            .values
            .get_mut(slot)
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid memory slot {slot}")))?;
        let Value::Int(current) = target else {
            return Err(TinyOneError::runtime(format!(
                "{op_name} expects integer operands"
            )));
        };
        *current =
            op(*current).ok_or_else(|| TinyOneError::runtime(format!("{op_name} overflow")))?;
        Ok(())
    }

    pub(crate) fn add_int(&mut self, slot: usize, value: i64) -> Result<()> {
        self.update_int_slot(slot, "Addition", |current| current.checked_add(value))
    }

    pub(crate) fn sub_int(&mut self, slot: usize, value: i64) -> Result<()> {
        self.update_int_slot(slot, "Subtraction", |current| current.checked_sub(value))
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.values.clone()
    }
}
