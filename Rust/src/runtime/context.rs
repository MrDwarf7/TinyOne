use std::collections::HashMap;

use crate::{Result, TinyHeap, TinyHeapStats, TinyOneError};

#[derive(Debug)]
pub(crate) struct TinyRuntimeContext {
    pub(crate) heap: TinyHeap,
    pub(crate) inputs: Vec<String>,
    pub(crate) input_index: usize,
    pub(crate) io_stdout: String,
    pub(crate) io_stderr: String,
    pub(crate) sys_args: Vec<String>,
    pub(crate) sys_env: HashMap<String, String>,
}

impl TinyRuntimeContext {
    pub(crate) fn new(inputs: impl IntoIterator<Item = String>) -> Self {
        Self {
            heap: TinyHeap::new(),
            inputs: inputs.into_iter().collect(),
            input_index: 0,
            io_stdout: String::new(),
            io_stderr: String::new(),
            sys_args: Vec::new(),
            sys_env: HashMap::new(),
        }
    }

    pub(crate) fn read_raw(&mut self) -> Result<String> {
        if self.input_index >= self.inputs.len() {
            return Err(TinyOneError::runtime("Input exhausted"));
        }
        let value = self
            .inputs
            .get(self.input_index)
            .cloned()
            .ok_or_else(|| TinyOneError::runtime("Input exhausted"))?;
        self.input_index += 1;
        Ok(value)
    }

    pub(crate) fn heap_stats(&self) -> TinyHeapStats {
        self.heap.stats()
    }

    pub(crate) fn shutdown(&mut self) -> TinyHeapStats {
        self.heap.shutdown()
    }

    pub(crate) fn set_sys_args(&mut self, args: Vec<String>) {
        self.sys_args = args;
    }

    pub(crate) fn set_sys_env(&mut self, env: HashMap<String, String>) {
        self.sys_env = env;
    }
}

impl Drop for TinyRuntimeContext {
    fn drop(&mut self) {
        self.shutdown();
    }
}
