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
    allocator: Arc<crate::tiny_allocator::TinyAllocator>,
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
    ///
    /// Shares the *same* `TinyAllocator` `Arc` the heap was wired with
    /// (`TinyHeap::allocator_handle`), rather than constructing a disconnected
    /// standalone instance — String/Buffer/CharBuffer heap objects now own
    /// their real Ralloc-backed memory directly, and a spawned thread's
    /// context must observe the same allocation-table bookkeeping as the
    /// main-thread context for that memory, not a separate, always-empty
    /// tracker. Falls back to a fresh standalone allocator only if the heap
    /// somehow has none wired (not expected via `TinyRuntimeContext::new`).
    pub(crate) fn with_heap(heap_arc: Arc<Mutex<TinyHeap>>) -> Self {
        let allocator = heap_arc
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allocator_handle()
            .unwrap_or_else(|| Arc::new(crate::tiny_allocator::TinyAllocator::with_defaults()));
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
            allocator,
        }
    }

    /// Return the [`TinyAllocator`] diagnostics layer for this context.
    ///
    /// This is always the same allocator instance wired into the heap
    /// (`TinyHeap::set_allocator`/`allocator_handle`), for both the primary
    /// context and thread-spawned contexts created via
    /// [`with_heap`][Self::with_heap].
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
