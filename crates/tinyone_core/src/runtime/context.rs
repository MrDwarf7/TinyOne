use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{Program, Result, TinyHeap, TinyHeapStats};

pub(crate) struct TinyRuntimeContext {
    pub(crate) heap_arc:      Arc<Mutex<TinyHeap>>,
    pub(crate) program_arc:   Option<Arc<Program>>,
    pub(crate) queued_stdout: Vec<u8>,
    pub(crate) inputs:        Vec<String>,
    pub(crate) input_index:   usize,
    pub(crate) io_stdout:     String,
    pub(crate) io_stderr:     String,
    pub(crate) sys_args:      Vec<String>,
    pub(crate) sys_env:       HashMap<String, String>,
    allocator:                Arc<crate::tiny_allocator::TinyAllocator>,
}

impl TinyRuntimeContext {
    pub(crate) fn new(inputs: impl IntoIterator<Item = String>) -> Self {
        let allocator = Arc::new(crate::tiny_allocator::TinyAllocator::with_defaults());
        let mut heap = TinyHeap::new();
        heap.set_allocator(Arc::clone(&allocator));
        Self {
            heap_arc: Arc::new(Mutex::new(heap)),
            program_arc: None,
            queued_stdout: Vec::new(),
            inputs: inputs.into_iter().collect(),
            input_index: 0,
            io_stdout: String::new(),
            io_stderr: String::new(),
            sys_args: Vec::new(),
            sys_env: HashMap::new(),
            allocator,
        }
    }

    /// Construct a context that shares an existing heap. Used by spawned threads.
    /// The heap already has its allocator wired; this context gets a reference to
    /// a standalone allocator for API compatibility. The heap-level allocator is
    /// the primary tracking layer for all heap operations.
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
            allocator: Arc::new(crate::tiny_allocator::TinyAllocator::with_defaults()),
        }
    }

    /// Return the [`TinyAllocator`] diagnostics layer for this context.
    ///
    /// For the primary (main-thread) context this is the same allocator that is
    /// wired into the heap.  For thread-spawned contexts (created via
    /// [`with_heap`]) this is a standalone instance; the heap's allocator is the
    /// authoritative tracker for all operations.
    ///
    /// [`with_heap`]: TinyRuntimeContext::with_heap
    pub fn allocator(&self) -> &crate::tiny_allocator::TinyAllocator {
        &self.allocator
    }

    /// Acquire the heap lock. Recovers from poisoning (a prior thread panicked
    /// while holding the lock) rather than aborting — the heap data structure
    /// is not torn across a Rust panic boundary at these call sites.
    #[inline]
    pub(crate) fn heap(&self) -> MutexGuard<'_, TinyHeap> {
        self.heap_arc.lock().unwrap_or_else(|e| e.into_inner())
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
        // Only shut down the heap when we're the last owner. strong_count
        // here still includes self (Arc fields destruct after Drop::drop),
        // so count > 1 means at least one peer context is still alive.
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
