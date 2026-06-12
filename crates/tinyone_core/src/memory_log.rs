//! Memory operation log for TinyOne diagnostics.
//!
//! This module provides a thread-safe ring-buffer log of VM memory operations.
//! It is intended purely for diagnostics and tooling — it does not affect the
//! correctness of the allocator or the VM.
//!
//! # Design constraints
//!
//! - All storage uses `std` allocation (Box/Vec/String), never the TinyOne heap.
//! - No operation panics; every method is best-effort and silently no-ops on error.
//! - Safe to call from allocator callbacks: the `Mutex` used here guards only
//!   `std`-allocated memory, so it can never re-enter the TinyOne heap.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

// ── OperationType ─────────────────────────────────────────────────────────────

/// The class of memory operation recorded in a [`MemoryLogEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    /// A fresh allocation was requested.
    Allocate,
    /// An allocation was released.
    Free,
    /// An existing allocation was resized.
    Realloc,
    /// A read through a VM pointer was performed.
    Read,
    /// A write through a VM pointer was performed.
    Write,
    /// A pointer or heap invariant was checked.
    Validate,
    /// An allocator-level error was detected.
    Error,
    /// The heap/VM is shutting down; final accounting entry.
    Shutdown,
}

// ── MemoryLogEntry ────────────────────────────────────────────────────────────

/// A single recorded memory operation.
#[derive(Debug, Clone)]
pub struct MemoryLogEntry {
    /// Monotonic counter; wrapping is acceptable for log purposes.
    pub seq: u64,

    /// VM logical thread id (not the OS thread id).
    pub thread_id: u64,

    /// The kind of operation that was performed.
    pub op_type: OperationType,

    /// The VM-side address involved in the operation.
    pub vm_address: usize,

    /// Generation counter of the heap object at `vm_address`, used to detect
    /// use-after-free across reuse of the same address slot.
    pub vm_generation: u64,

    /// Native allocator id, reserved for future Ralloc integration.
    /// Always `None` until that integration lands.
    pub native_alloc_id: Option<u64>,

    /// Number of bytes that were requested by the caller.
    pub requested_size: usize,

    /// Number of bytes that were actually allocated/affected (may differ from
    /// `requested_size` due to alignment padding or allocator rounding).
    pub effective_size: usize,

    /// Alignment (in bytes) that was requested or in effect.
    pub alignment: usize,

    /// `true` if the operation succeeded; `false` if it failed.
    pub result: bool,

    /// Human-readable error class when `result` is `false`, e.g. `"out_of_memory"`.
    pub error_class: Option<String>,
}

impl MemoryLogEntry {
    /// Construct a successful log entry.
    ///
    /// `size` is used for both `requested_size` and `effective_size`; callers
    /// that know the effective size can overwrite the fields after construction.
    pub fn success(seq: u64, thread_id: u64, op: OperationType, addr: usize, generation: u64, size: usize) -> Self {
        Self {
            seq,
            thread_id,
            op_type: op,
            vm_address: addr,
            vm_generation: generation,
            native_alloc_id: None,
            requested_size: size,
            effective_size: size,
            alignment: 0,
            result: true,
            error_class: None,
        }
    }

    /// Construct a failed log entry.
    ///
    /// `size` fields default to `0`; `error` becomes the `error_class`.
    pub fn failure(
        seq: u64,
        thread_id: u64,
        op: OperationType,
        addr: usize,
        generation: u64,
        error: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            thread_id,
            op_type: op,
            vm_address: addr,
            vm_generation: generation,
            native_alloc_id: None,
            requested_size: 0,
            effective_size: 0,
            alignment: 0,
            result: false,
            error_class: Some(error.into()),
        }
    }
}

// ── MemoryLog ─────────────────────────────────────────────────────────────────

/// Thread-safe ring-buffer of [`MemoryLogEntry`] records.
///
/// The buffer has a fixed maximum capacity; when it is full, the oldest entry
/// is silently dropped to make room for the newest one.
///
/// All methods are infallible and best-effort: if an internal lock is poisoned
/// the call silently no-ops rather than panicking.
pub struct MemoryLog {
    /// Ring buffer and associated write head; guarded by a single `Mutex`.
    ///
    /// We hold both the buffer and the write position inside the same lock so
    /// that `log()` is a single critical section and can never deadlock against
    /// itself regardless of call site.
    inner: Mutex<LogInner>,

    /// Whether logging is currently active. Checked before acquiring `inner`
    /// so disabled logging is almost free.
    enabled: AtomicBool,
}

struct LogInner {
    /// Fixed-capacity ring buffer pre-allocated at construction time.
    buf:      Vec<Option<MemoryLogEntry>>,
    /// Capacity (== `buf.len()`).
    capacity: usize,
    /// Index of the *next* write slot (wraps at `capacity`).
    head:     usize,
    /// Number of entries currently stored (saturates at `capacity`).
    len:      usize,
}

impl LogInner {
    fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1); // capacity 0 is nonsensical; clamp to 1
        let mut buf = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buf.push(None);
        }
        Self {
            buf,
            capacity,
            head: 0,
            len: 0,
        }
    }

    /// Append one entry, evicting the oldest if the buffer is full.
    fn push(&mut self, entry: MemoryLogEntry) {
        self.buf[self.head] = Some(entry);
        self.head = (self.head + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        }
        // When len == capacity the write just overwrote the oldest slot, which
        // is exactly the desired ring-buffer eviction behaviour.
    }

    /// Return all entries in insertion order (oldest first).
    fn snapshot(&self) -> Vec<MemoryLogEntry> {
        let mut out = Vec::with_capacity(self.len);
        if self.len == 0 {
            return out;
        }
        // If the buffer is not yet full `head` is also the oldest slot (index 0 up
        // to head-1 are valid). If it is full the oldest entry sits at `head`.
        let start = if self.len < self.capacity {
            0
        } else {
            self.head // head points to the *next* write == oldest entry
        };
        for i in 0..self.len {
            let idx = (start + i) % self.capacity;
            if let Some(e) = &self.buf[idx] {
                out.push(e.clone());
            }
        }
        out
    }

    fn clear(&mut self) {
        for slot in &mut self.buf {
            *slot = None;
        }
        self.head = 0;
        self.len = 0;
    }
}

/// Default ring-buffer capacity used by [`MemoryLog::with_default_capacity`].
pub const DEFAULT_LOG_CAPACITY: usize = 1024;

impl MemoryLog {
    /// Create a new log with the given ring-buffer capacity.
    ///
    /// A capacity of `0` is normalised to `1` internally.
    /// Logging is **enabled** by default.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner:   Mutex::new(LogInner::new(capacity)),
            enabled: AtomicBool::new(true),
        }
    }

    /// Create a new log with the default capacity of 1 024 entries.
    ///
    /// Logging is **enabled** by default.
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_LOG_CAPACITY)
    }

    /// Append `entry` to the log.
    ///
    /// If the buffer is full the oldest entry is silently evicted.
    /// If logging is disabled or the internal lock is poisoned, this is a no-op.
    pub fn log(&self, entry: MemoryLogEntry) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        // Ignore poisoned mutex — diagnostics must never panic the VM.
        if let Ok(mut inner) = self.inner.lock() {
            inner.push(entry);
        }
    }

    /// Return a snapshot of all current entries in insertion order (oldest first).
    ///
    /// Returns an empty `Vec` if logging is disabled or the lock is poisoned.
    pub fn snapshot(&self) -> Vec<MemoryLogEntry> {
        match self.inner.lock() {
            Ok(inner) => inner.snapshot(),
            Err(_) => Vec::new(),
        }
    }

    /// Remove all entries from the log.
    ///
    /// Silently no-ops if the lock is poisoned.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.clear();
        }
    }

    /// Enable logging (the default state after construction).
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// Disable logging.  Subsequent calls to [`log`](Self::log) are no-ops
    /// until [`enable`](Self::enable) is called again.
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }

    /// Returns `true` if logging is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Returns the number of entries currently stored in the log.
    ///
    /// Returns `0` if the lock is poisoned.
    pub fn len(&self) -> usize {
        match self.inner.lock() {
            Ok(inner) => inner.len,
            Err(_) => 0,
        }
    }

    /// Returns `true` if the log contains no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn make_entry(seq: u64) -> MemoryLogEntry {
        MemoryLogEntry::success(seq, 0, OperationType::Allocate, 0x1000, 1, 64)
    }

    #[test]
    fn new_log_is_empty() {
        let log = MemoryLog::with_default_capacity();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert!(log.is_enabled());
    }

    #[test]
    fn single_entry_round_trip() {
        let log = MemoryLog::with_default_capacity();
        log.log(make_entry(1));
        let snap = log.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].seq, 1);
    }

    #[test]
    fn ring_evicts_oldest_when_full() {
        let log = MemoryLog::new(4);
        for i in 0..6u64 {
            log.log(make_entry(i));
        }
        assert_eq!(log.len(), 4);
        let snap = log.snapshot();
        // Oldest surviving entry should be seq=2 (0 and 1 were evicted).
        assert_eq!(snap[0].seq, 2);
        assert_eq!(snap[3].seq, 5);
    }

    #[test]
    fn clear_empties_log() {
        let log = MemoryLog::with_default_capacity();
        log.log(make_entry(1));
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.snapshot().len(), 0);
    }

    #[test]
    fn disable_suppresses_logging() {
        let log = MemoryLog::with_default_capacity();
        log.disable();
        assert!(!log.is_enabled());
        log.log(make_entry(1));
        assert!(log.is_empty());
        log.enable();
        log.log(make_entry(2));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn success_entry_fields() {
        let e = MemoryLogEntry::success(7, 3, OperationType::Write, 0xdead, 5, 128);
        assert_eq!(e.seq, 7);
        assert_eq!(e.thread_id, 3);
        assert_eq!(e.op_type, OperationType::Write);
        assert_eq!(e.vm_address, 0xdead);
        assert_eq!(e.vm_generation, 5);
        assert_eq!(e.requested_size, 128);
        assert_eq!(e.effective_size, 128);
        assert!(e.result);
        assert!(e.error_class.is_none());
        assert!(e.native_alloc_id.is_none());
    }

    #[test]
    fn failure_entry_fields() {
        let e = MemoryLogEntry::failure(9, 1, OperationType::Allocate, 0, 0, "out_of_memory");
        assert_eq!(e.seq, 9);
        assert!(!e.result);
        assert_eq!(e.error_class.as_deref(), Some("out_of_memory"));
        assert_eq!(e.requested_size, 0);
    }

    #[test]
    fn concurrent_logging_no_data_races() {
        use std::thread;

        let log = Arc::new(MemoryLog::new(256));
        let mut handles = Vec::new();

        for t in 0..8u64 {
            let log = Arc::clone(&log);
            handles.push(thread::spawn(move || {
                for i in 0..50u64 {
                    log.log(MemoryLogEntry::success(t * 50 + i, t, OperationType::Allocate, 0, 0, 8));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // 8 threads × 50 entries = 400 total; ring is 256 so we expect exactly 256.
        assert_eq!(log.len(), 256);
    }

    #[test]
    fn snapshot_order_is_oldest_first() {
        let log = MemoryLog::new(4);
        // Fill exactly to capacity.
        for i in 0..4u64 {
            log.log(make_entry(i));
        }
        let snap = log.snapshot();
        assert_eq!(snap.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![0, 1, 2, 3]);

        // Overwrite one entry, oldest (0) evicted → [1,2,3,4].
        log.log(make_entry(4));
        let snap = log.snapshot();
        assert_eq!(snap.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    }
}
