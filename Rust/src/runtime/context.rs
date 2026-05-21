use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{Program, Result, TinyHeap, TinyHeapStats};

pub(crate) struct TinyRuntimeContext {
    pub(crate) heap_arc: Arc<Mutex<TinyHeap>>,
    pub(crate) program_arc: Option<Arc<Program>>,
    pub(crate) queued_stdout: Vec<u8>,
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
            heap_arc: Arc::new(Mutex::new(TinyHeap::new())),
            program_arc: None,
            queued_stdout: Vec::new(),
            inputs: inputs.into_iter().collect(),
            input_index: 0,
            io_stdout: String::new(),
            io_stderr: String::new(),
            sys_args: Vec::new(),
            sys_env: HashMap::new(),
        }
    }

    /// Construct a context that shares an existing heap. Used by spawned threads.
    pub(crate) fn with_heap(heap_arc: Arc<Mutex<TinyHeap>>) -> Self {
        Self {
            heap_arc,
            program_arc: None,
            queued_stdout: Vec::new(),
            inputs: Vec::new(),
            input_index: 0,
            io_stdout: String::new(),
            io_stderr: String::new(),
            sys_args: Vec::new(),
            sys_env: HashMap::new(),
        }
    }

    /// Acquire the heap lock.
    #[inline]
    pub(crate) fn heap(&self) -> MutexGuard<'_, TinyHeap> {
        self.heap_arc.lock().expect("heap mutex poisoned")
    }

    pub(crate) fn read_raw(&mut self) -> Result<String> {
        if self.input_index >= self.inputs.len() {
            return Err(crate::TinyOneError::runtime("Input exhausted"));
        }
        let value = self.inputs[self.input_index].clone();
        self.input_index += 1;
        Ok(value)
    }

    pub(crate) fn heap_stats(&self) -> TinyHeapStats {
        self.heap().stats()
    }

    pub(crate) fn shutdown(&mut self) -> TinyHeapStats {
        self.heap().shutdown()
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
        // Only shut down the heap when we're the last owner.
        // Thread contexts share the heap with the main context —
        // calling shutdown on a shared heap would free all objects
        // while the main context is still running.
        if Arc::strong_count(&self.heap_arc) == 1 {
            self.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_heap_arc_is_shared_across_clones() {
        let ctx1 = TinyRuntimeContext::new(Vec::<String>::new());
        let ctx2 = TinyRuntimeContext::with_heap(Arc::clone(&ctx1.heap_arc));
        let _hr = ctx1.heap().alloc_string("hello").unwrap();
        assert_eq!(ctx2.heap().stats().live_objects, 1);
    }
}
