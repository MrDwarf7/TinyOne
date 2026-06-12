//! VM memory hook and telemetry interface.
//!
//! This module provides a pluggable hook system that external diagnostic tools
//! (tests, profilers, error reporters) can use to receive memory events emitted
//! by the TinyOne VM.
//!
//! # Design
//! - [`MemoryEvent`] describes every class of memory activity the VM can surface.
//! - [`VmMemoryHook`] is the trait implementors register.
//! - [`HookRegistry`] is the thread-safe hook list that dispatches events.
//! - [`MemoryErrorPusher`] is a ready-made hook for collecting error events.
//!
//! # Thread safety
//! All public types are `Send + Sync` and are designed to be shared via [`Arc`].

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, RwLock};

// ---------------------------------------------------------------------------
// MemoryEvent
// ---------------------------------------------------------------------------

/// Describes a memory-related event that occurred inside the TinyOne VM.
///
/// The `vm_address` and `vm_generation` fields use the VM's internal heap
/// addressing scheme — they are not raw host pointers.
#[derive(Debug, Clone)]
pub enum MemoryEvent {
    /// A new heap object was allocated.
    Allocated {
        /// VM-internal address of the allocated object.
        vm_address:    usize,
        /// Heap generation counter at the time of allocation.
        vm_generation: u64,
        /// Size of the allocation in bytes.
        size:          usize,
        /// Canonical type name of the allocated object.
        type_name:     &'static str,
    },

    /// A heap object was freed.
    Freed {
        /// VM-internal address of the freed object.
        vm_address:    usize,
        /// Heap generation counter at the time of the free.
        vm_generation: u64,
    },

    /// A heap object was reallocated (resized in place or moved).
    Reallocated {
        /// VM-internal address of the object after reallocation.
        vm_address:    usize,
        /// Heap generation counter at the time of reallocation.
        vm_generation: u64,
        /// Size of the object before reallocation, in bytes.
        old_size:      usize,
        /// Size of the object after reallocation, in bytes.
        new_size:      usize,
    },

    /// An access violation was attempted (bounds check, null dereference, etc.).
    AccessViolation {
        /// VM-internal address that was accessed illegally.
        vm_address:    usize,
        /// Heap generation counter at the time of the violation.
        vm_generation: u64,
        /// The operation that triggered the violation (e.g. `"load"`, `"store"`).
        operation:     &'static str,
        /// Human-readable reason for the violation.
        reason:        &'static str,
    },

    /// The allocator ran out of memory.
    OutOfMemory {
        /// Number of bytes that were requested when the failure occurred.
        requested_size: usize,
        /// Total live bytes at the time of the failure.
        live_bytes:     usize,
        /// Configured heap byte limit.
        limit_bytes:    usize,
    },

    /// A stale or dangling pointer was detected.
    StalePointer {
        /// VM-internal address of the stale pointer.
        vm_address:   usize,
        /// Generation the pointer was created in.
        expected_gen: u64,
        /// Generation currently recorded for the slot.
        actual_gen:   u64,
    },

    /// A double-free was attempted.
    DoubleFree {
        /// VM-internal address of the object that was freed twice.
        vm_address:    usize,
        /// Heap generation counter at the time of the second free attempt.
        vm_generation: u64,
    },

    /// A buffer out-of-bounds read or write was attempted.
    BufferOverflow {
        /// VM-internal address of the buffer.
        vm_address:    usize,
        /// Heap generation counter at the time of the access.
        vm_generation: u64,
        /// Index that was out of range.
        index:         usize,
        /// Current length of the buffer.
        len:           usize,
    },

    /// VM shutdown — reports final live allocation counts.
    ShutdownDrain {
        /// Number of live objects remaining at shutdown.
        live_count: usize,
        /// Total live bytes remaining at shutdown.
        live_bytes: usize,
    },

    /// An allocator-level error that is not tied to a specific VM pointer.
    AllocatorError {
        /// Human-readable description of the error.
        message: String,
    },
}

impl MemoryEvent {
    /// Returns `true` if this event represents a memory safety violation or
    /// allocator failure — i.e. something that should be surfaced as an error.
    ///
    /// Non-error events (`Allocated`, `Freed`, `Reallocated`, `ShutdownDrain`)
    /// return `false`.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            MemoryEvent::AccessViolation { .. }
                | MemoryEvent::OutOfMemory { .. }
                | MemoryEvent::StalePointer { .. }
                | MemoryEvent::DoubleFree { .. }
                | MemoryEvent::BufferOverflow { .. }
                | MemoryEvent::AllocatorError { .. }
        )
    }

    /// Returns a short, human-readable name for the event kind, suitable for
    /// log labels and metrics tags.
    pub fn kind_name(&self) -> &'static str {
        match self {
            MemoryEvent::Allocated { .. } => "Allocated",
            MemoryEvent::Freed { .. } => "Freed",
            MemoryEvent::Reallocated { .. } => "Reallocated",
            MemoryEvent::AccessViolation { .. } => "AccessViolation",
            MemoryEvent::OutOfMemory { .. } => "OutOfMemory",
            MemoryEvent::StalePointer { .. } => "StalePointer",
            MemoryEvent::DoubleFree { .. } => "DoubleFree",
            MemoryEvent::BufferOverflow { .. } => "BufferOverflow",
            MemoryEvent::ShutdownDrain { .. } => "ShutdownDrain",
            MemoryEvent::AllocatorError { .. } => "AllocatorError",
        }
    }
}

// ---------------------------------------------------------------------------
// VmMemoryHook trait
// ---------------------------------------------------------------------------

/// A hook that receives memory events from the TinyOne VM.
///
/// Implementors must be `Send + Sync` so that they can be shared safely across
/// VM threads. Register hooks with [`HookRegistry::register`].
///
/// # Panic safety
/// Implementors SHOULD NOT panic inside `on_memory_event`. The registry
/// catches panics via [`std::panic::catch_unwind`] and logs them to stderr,
/// but a panicking hook is otherwise silently skipped for that dispatch.
pub trait VmMemoryHook: Send + Sync {
    /// Called for every [`MemoryEvent`] dispatched through the registry.
    fn on_memory_event(&self, event: &MemoryEvent);
}

// ---------------------------------------------------------------------------
// HookRegistry
// ---------------------------------------------------------------------------

/// Thread-safe registry of [`VmMemoryHook`]s.
///
/// Hooks are called in registration order. A panicking hook is caught and
/// logged to stderr; it does not affect other hooks or crash the VM.
///
/// `HookRegistry` is `Send + Sync` and is intended to be shared via [`Arc`].
pub struct HookRegistry {
    hooks: RwLock<Vec<Arc<dyn VmMemoryHook>>>,
}

impl HookRegistry {
    /// Create a new, empty `HookRegistry`.
    pub fn new() -> Self {
        Self {
            hooks: RwLock::new(Vec::new()),
        }
    }

    /// Register a hook. Hooks are called in registration order during dispatch.
    ///
    /// This acquires a write lock on the hook list briefly, so it is
    /// safe to call while no dispatch is in progress (or while another thread
    /// is mid-dispatch — the write will wait until all readers finish).
    pub fn register(&self, hook: Arc<dyn VmMemoryHook>) {
        self.hooks
            .write()
            .expect("HookRegistry RwLock poisoned on register")
            .push(hook);
    }

    /// Remove all registered hooks.
    pub fn clear(&self) {
        self.hooks
            .write()
            .expect("HookRegistry RwLock poisoned on clear")
            .clear();
    }

    /// Dispatch a [`MemoryEvent`] to all registered hooks.
    ///
    /// # Panic safety
    /// Each hook is invoked inside [`std::panic::catch_unwind`]. A hook that
    /// panics is skipped for this event and a message is printed to stderr.
    /// The panic is **not** propagated — the VM remains stable.
    pub fn dispatch(&self, event: MemoryEvent) {
        let hooks = self.hooks.read().expect("HookRegistry RwLock poisoned on dispatch");

        for hook in hooks.iter() {
            // Clone the Arc so we can move it (and &event) into the closure
            // without borrowing `hooks` across the unwind boundary.
            let hook_ref = Arc::clone(hook);
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                hook_ref.on_memory_event(&event);
            }));

            if let Err(panic_payload) = result {
                // Produce a best-effort message without touching the TinyOne heap.
                let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    format!("[vm_hooks] hook panicked: {s}")
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    format!("[vm_hooks] hook panicked: {s}")
                } else {
                    "[vm_hooks] hook panicked with an opaque payload".to_owned()
                };
                eprintln!("{msg}");
            }
        }
    }

    /// Returns `true` if no hooks are currently registered.
    pub fn is_empty(&self) -> bool {
        self.hooks
            .read()
            .expect("HookRegistry RwLock poisoned on is_empty")
            .is_empty()
    }

    /// Returns the number of currently registered hooks.
    pub fn hook_count(&self) -> usize {
        self.hooks
            .read()
            .expect("HookRegistry RwLock poisoned on hook_count")
            .len()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: HookRegistry contains only an RwLock<Vec<Arc<dyn VmMemoryHook>>>.
// RwLock is Send+Sync when its contents are Send.
// Arc<dyn VmMemoryHook> is Send+Sync because VmMemoryHook: Send+Sync.
// Therefore HookRegistry is Send+Sync.

// ---------------------------------------------------------------------------
// MemoryErrorPusher
// ---------------------------------------------------------------------------

/// A concrete [`VmMemoryHook`] that collects memory *error* events in a
/// bounded, thread-safe queue.
///
/// Only events for which [`MemoryEvent::is_error`] returns `true` are stored.
/// When the queue is at capacity, the oldest event is dropped to make room
/// (FIFO eviction).
///
/// Intended for use in tests and error-monitoring integrations.
///
/// # Example
/// ```rust,ignore
/// let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
/// registry.register(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);
/// // ... run some VM code ...
/// let errors = pusher.drain_errors();
/// assert!(errors.is_empty(), "unexpected memory errors: {errors:?}");
/// ```
pub struct MemoryErrorPusher {
    /// Maximum number of error events retained in the queue.
    capacity: usize,
    /// The bounded error queue.
    /// `VecDeque` gives O(1) front/back push-pop for FIFO eviction.
    queue:    Mutex<std::collections::VecDeque<MemoryEvent>>,
}

impl MemoryErrorPusher {
    /// Default error queue capacity.
    const DEFAULT_CAPACITY: usize = 256;

    /// Create a new `MemoryErrorPusher` with the given error queue capacity.
    ///
    /// When the queue reaches `capacity`, the oldest stored event is evicted
    /// before the new event is enqueued.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: Mutex::new(std::collections::VecDeque::with_capacity(capacity)),
        }
    }

    /// Create a `MemoryErrorPusher` with the default capacity (256 events).
    pub fn with_default_capacity() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }

    /// Drain all stored error events, emptying the internal queue.
    ///
    /// Returns the events in the order they were received (oldest first).
    pub fn drain_errors(&self) -> Vec<MemoryEvent> {
        let mut queue = self
            .queue
            .lock()
            .expect("MemoryErrorPusher Mutex poisoned on drain_errors");
        queue.drain(..).collect()
    }

    /// Peek at the current number of queued error events without draining.
    pub fn error_count(&self) -> usize {
        self.queue
            .lock()
            .expect("MemoryErrorPusher Mutex poisoned on error_count")
            .len()
    }

    /// Returns `true` if any error events are currently queued.
    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }
}

impl VmMemoryHook for MemoryErrorPusher {
    /// Enqueues the event if [`MemoryEvent::is_error`] returns `true`.
    ///
    /// When the queue is at capacity, the oldest event is evicted (FIFO) to
    /// make room for the new one, so recent errors are always preserved.
    fn on_memory_event(&self, event: &MemoryEvent) {
        if !event.is_error() {
            return;
        }

        let mut queue = self
            .queue
            .lock()
            .expect("MemoryErrorPusher Mutex poisoned on on_memory_event");

        if queue.len() >= self.capacity {
            queue.pop_front(); // evict oldest to stay within capacity
        }
        queue.push_back(event.clone());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // A simple hook that counts how many events it receives.
    struct CountingHook {
        count: Mutex<usize>,
    }

    impl CountingHook {
        fn new() -> Arc<Self> {
            Arc::new(Self { count: Mutex::new(0) })
        }

        fn count(&self) -> usize {
            *self.count.lock().unwrap()
        }
    }

    impl VmMemoryHook for CountingHook {
        fn on_memory_event(&self, _event: &MemoryEvent) {
            *self.count.lock().unwrap() += 1;
        }
    }

    // A hook that always panics.
    struct PanickingHook;

    impl VmMemoryHook for PanickingHook {
        fn on_memory_event(&self, _event: &MemoryEvent) {
            panic!("intentional hook panic");
        }
    }

    fn alloc_event() -> MemoryEvent {
        MemoryEvent::Allocated {
            vm_address:    0x1000,
            vm_generation: 1,
            size:          64,
            type_name:     "Array",
        }
    }

    fn oom_event() -> MemoryEvent {
        MemoryEvent::OutOfMemory {
            requested_size: 1024,
            live_bytes:     900,
            limit_bytes:    1000,
        }
    }

    // --- MemoryEvent tests ---

    #[test]
    fn test_is_error_for_non_error_events() {
        assert!(!alloc_event().is_error());
        assert!(
            !MemoryEvent::Freed {
                vm_address:    0x1000,
                vm_generation: 1,
            }
            .is_error()
        );
        assert!(
            !MemoryEvent::Reallocated {
                vm_address:    0x1000,
                vm_generation: 1,
                old_size:      64,
                new_size:      128,
            }
            .is_error()
        );
        assert!(
            !MemoryEvent::ShutdownDrain {
                live_count: 0,
                live_bytes: 0,
            }
            .is_error()
        );
    }

    #[test]
    fn test_is_error_for_error_events() {
        assert!(oom_event().is_error());
        assert!(
            MemoryEvent::AccessViolation {
                vm_address:    0x1000,
                vm_generation: 1,
                operation:     "load",
                reason:        "null deref",
            }
            .is_error()
        );
        assert!(
            MemoryEvent::StalePointer {
                vm_address:   0x1000,
                expected_gen: 1,
                actual_gen:   2,
            }
            .is_error()
        );
        assert!(
            MemoryEvent::DoubleFree {
                vm_address:    0x1000,
                vm_generation: 1,
            }
            .is_error()
        );
        assert!(
            MemoryEvent::BufferOverflow {
                vm_address:    0x1000,
                vm_generation: 1,
                index:         10,
                len:           5,
            }
            .is_error()
        );
        assert!(MemoryEvent::AllocatorError { message: "oom".into() }.is_error());
    }

    #[test]
    fn test_kind_name() {
        assert_eq!(alloc_event().kind_name(), "Allocated");
        assert_eq!(oom_event().kind_name(), "OutOfMemory");
        assert_eq!(
            MemoryEvent::ShutdownDrain {
                live_count: 0,
                live_bytes: 0,
            }
            .kind_name(),
            "ShutdownDrain"
        );
    }

    // --- HookRegistry tests ---

    #[test]
    fn test_registry_starts_empty() {
        let reg = HookRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.hook_count(), 0);
    }

    #[test]
    fn test_register_and_count() {
        let reg = HookRegistry::new();
        reg.register(CountingHook::new() as Arc<dyn VmMemoryHook>);
        reg.register(CountingHook::new() as Arc<dyn VmMemoryHook>);
        assert!(!reg.is_empty());
        assert_eq!(reg.hook_count(), 2);
    }

    #[test]
    fn test_clear() {
        let reg = HookRegistry::new();
        reg.register(CountingHook::new() as Arc<dyn VmMemoryHook>);
        reg.clear();
        assert!(reg.is_empty());
    }

    #[test]
    fn test_dispatch_reaches_hooks() {
        let reg = HookRegistry::new();
        let hook = CountingHook::new();
        reg.register(Arc::clone(&hook) as Arc<dyn VmMemoryHook>);
        reg.dispatch(alloc_event());
        reg.dispatch(oom_event());
        assert_eq!(hook.count(), 2);
    }

    #[test]
    fn test_dispatch_panicking_hook_does_not_crash() {
        let reg = HookRegistry::new();
        let counter = CountingHook::new();
        // Register panicking hook first, then a counting hook.
        reg.register(Arc::new(PanickingHook) as Arc<dyn VmMemoryHook>);
        reg.register(Arc::clone(&counter) as Arc<dyn VmMemoryHook>);
        // Must not panic; the counting hook should still be called.
        reg.dispatch(alloc_event());
        assert_eq!(counter.count(), 1);
    }

    // --- MemoryErrorPusher tests ---

    #[test]
    fn test_pusher_ignores_non_error_events() {
        let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
        let reg = HookRegistry::new();
        reg.register(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);
        reg.dispatch(alloc_event()); // not an error
        assert!(!pusher.has_errors());
        assert_eq!(pusher.error_count(), 0);
    }

    #[test]
    fn test_pusher_collects_error_events() {
        let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
        let reg = HookRegistry::new();
        reg.register(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);
        reg.dispatch(oom_event());
        assert!(pusher.has_errors());
        assert_eq!(pusher.error_count(), 1);
    }

    #[test]
    fn test_pusher_drain_empties_queue() {
        let pusher = MemoryErrorPusher::with_default_capacity();
        pusher.on_memory_event(&oom_event());
        pusher.on_memory_event(&oom_event());
        let drained = pusher.drain_errors();
        assert_eq!(drained.len(), 2);
        assert_eq!(pusher.error_count(), 0);
    }

    #[test]
    fn test_pusher_bounded_capacity_evicts_oldest() {
        let pusher = MemoryErrorPusher::new(3);
        // Fill to capacity + 1
        for i in 0..4u64 {
            pusher.on_memory_event(&MemoryEvent::StalePointer {
                vm_address:   i as usize,
                expected_gen: 0,
                actual_gen:   1,
            });
        }
        assert_eq!(pusher.error_count(), 3);
        let events = pusher.drain_errors();
        // The oldest (vm_address 0) should have been evicted.
        if let MemoryEvent::StalePointer { vm_address, .. } = &events[0] {
            assert_eq!(*vm_address, 1);
        } else {
            panic!("unexpected event type");
        }
    }

    #[test]
    fn test_pusher_default_capacity_is_256() {
        let pusher = MemoryErrorPusher::with_default_capacity();
        // Fill exactly to capacity — none should be evicted.
        for _ in 0..256 {
            pusher.on_memory_event(&oom_event());
        }
        assert_eq!(pusher.error_count(), 256);
        // One more should evict the oldest.
        pusher.on_memory_event(&oom_event());
        assert_eq!(pusher.error_count(), 256);
    }

    #[test]
    fn test_registry_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<HookRegistry>();
    }

    #[test]
    fn test_pusher_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MemoryErrorPusher>();
    }
}
