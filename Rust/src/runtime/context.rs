use crate::{Result, TinyHeap, TinyHeapStats, TinyOneError};

#[derive(Debug)]
pub(crate) struct TinyRuntimeContext {
    pub(crate) heap: TinyHeap,
    pub(crate) inputs: Vec<String>,
    pub(crate) input_index: usize,
}

impl TinyRuntimeContext {
    pub(crate) fn new(inputs: impl IntoIterator<Item = String>) -> Self {
        Self {
            heap: TinyHeap::new(),
            inputs: inputs.into_iter().collect(),
            input_index: 0,
        }
    }

    pub(crate) fn read_raw(&mut self) -> Result<String> {
        if self.input_index >= self.inputs.len() {
            return Err(TinyOneError::runtime("Input exhausted"));
        }
        let value = self.inputs[self.input_index].clone();
        self.input_index += 1;
        Ok(value)
    }

    pub(crate) fn heap_stats(&self) -> TinyHeapStats {
        self.heap.stats()
    }

    pub(crate) fn shutdown(&mut self) -> TinyHeapStats {
        self.heap.shutdown()
    }
}

impl Drop for TinyRuntimeContext {
    fn drop(&mut self) {
        self.shutdown();
    }
}
