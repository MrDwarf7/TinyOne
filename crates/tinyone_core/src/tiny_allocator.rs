//! `TinyAllocator` — the boundary layer between TinyOne's VM heap and Ralloc.
//!
//! This module owns:
//! - An [`AllocTable`] — the live-allocation registry keyed by `vm_address`.
//! - A [`MemoryLog`] — a bounded ring-buffer of operation records for diagnostics.
//! - A [`HookRegistry`] — dispatches [`MemoryEvent`]s to registered observers.
//! - A sequence counter and shutdown flag.
//!
//! # Phase 2 vs Phase 3
//! All Ralloc interaction is stubbed out in this phase.  Placeholder types are
//! marked with `// PHASE3: replace with VmAllocation` so they are easy to audit
//! before Phase 3 integration.
//!
//! # Thread safety
//! [`TinyAllocator`] is `Send + Sync`.  Interior mutability is managed by the
//! locks embedded in [`AllocTable`], [`MemoryLog`], and [`HookRegistry`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::alloc_table::{AllocKind, AllocRecord, AllocTable, AllocTableError, AllocTableStats, VmAllocHandle};
use crate::memory_log::{MemoryLog, MemoryLogEntry, OperationType};
use crate::vm_hooks::{HookRegistry, MemoryEvent, VmMemoryHook};

// ── AllocKind helpers ─────────────────────────────────────────────────────────

impl AllocKind {
    /// Returns a canonical human-readable type name for this allocation kind.
    ///
    /// Used internally when building [`MemoryEvent::Allocated`] payloads.
    fn type_name(self) -> &'static str {
        match self {
            AllocKind::String => "String",
            AllocKind::Array => "Array",
            AllocKind::Buffer => "Buffer",
            AllocKind::Struct => "Struct",
            AllocKind::Cell => "Cell",
            AllocKind::Map => "Map",
            AllocKind::Mutex => "Mutex",
            AllocKind::Atomic => "Atomic",
            AllocKind::Thread => "Thread",
            AllocKind::Char => "Char",
            AllocKind::CharBuffer => "CharBuffer",
            AllocKind::Vec => "Vec",
            AllocKind::Record => "Record",
            AllocKind::Dictionary => "Dictionary",
            AllocKind::Box => "Box",
            AllocKind::Raw => "Raw",
            AllocKind::Closure => "Closure",
            AllocKind::Sum => "Sum",
            AllocKind::Enum => "Enum",
            AllocKind::TaggedUnion => "TaggedUnion",
            AllocKind::Result => "Result",
            AllocKind::Option => "Option",
            AllocKind::Dyn => "Dyn",
            AllocKind::FileDescriptor => "FileDescriptor",
        }
    }
}

// ── TinyAllocatorError ────────────────────────────────────────────────────────

/// Errors that can be returned by [`TinyAllocator`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TinyAllocatorError {
    /// The allocator could not satisfy the request: `requested` bytes were
    /// needed but only `live` bytes are currently used against a `limit`.
    OutOfMemory {
        /// Bytes the caller asked for.
        requested: usize,
        /// Live bytes in the table at the time of the failure.
        live:      usize,
        /// Configured byte limit.
        limit:     usize,
    },
    /// The requested size was not valid (e.g. zero when zero-sized types are
    /// not supported by the backend).
    InvalidSize {
        /// The size value that was rejected.
        size: usize,
    },
    /// The requested alignment is not a power of two or is otherwise invalid.
    InvalidAlignment {
        /// The alignment value that was rejected.
        align: usize,
    },
    /// The [`AllocTable`] is at capacity and cannot accept another record.
    AllocationTableFull,
    /// No live allocation exists at `vm_address`.
    NotFound {
        /// The VM address that was not found.
        vm_address: usize,
    },
    /// An allocation exists at `vm_address` but its generation does not match.
    GenerationMismatch {
        /// The VM address that was looked up.
        vm_address:   usize,
        /// The generation the caller expected.
        expected_gen: u64,
        /// The generation recorded in the table.
        actual_gen:   u64,
    },
    /// `free` was called on an address that was already freed.
    DoubleFree {
        /// The VM address that was freed twice.
        vm_address: usize,
    },
    /// The native (Ralloc) allocator returned an error.
    NativeAllocFailed,
    /// [`TinyAllocator::shutdown_drain`] has already been called; no new
    /// allocations are accepted.
    ShutdownInProgress,
}

impl TinyAllocatorError {
    /// Returns `true` if this error represents a memory *safety* violation
    /// (rather than a resource-exhaustion condition like OOM).
    ///
    /// Safety violations are: [`GenerationMismatch`], [`DoubleFree`], and
    /// [`NativeAllocFailed`] (which implies heap corruption potential).
    ///
    /// [`GenerationMismatch`]: TinyAllocatorError::GenerationMismatch
    /// [`DoubleFree`]: TinyAllocatorError::DoubleFree
    /// [`NativeAllocFailed`]: TinyAllocatorError::NativeAllocFailed
    pub fn is_safety_violation(&self) -> bool {
        matches!(
            self,
            TinyAllocatorError::GenerationMismatch { .. }
                | TinyAllocatorError::DoubleFree { .. }
                | TinyAllocatorError::NativeAllocFailed
        )
    }
}

impl std::fmt::Display for TinyAllocatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TinyAllocatorError::OutOfMemory { requested, live, limit } => {
                write!(f, "out of memory: requested {requested} bytes (live={live}, limit={limit})")
            }
            TinyAllocatorError::InvalidSize { size } => {
                write!(f, "invalid allocation size: {size}")
            }
            TinyAllocatorError::InvalidAlignment { align } => {
                write!(f, "invalid alignment: {align} (must be a power of two)")
            }
            TinyAllocatorError::AllocationTableFull => {
                write!(f, "allocation table is full; cannot insert new record")
            }
            TinyAllocatorError::NotFound { vm_address } => {
                write!(f, "no live allocation found at vm_address {vm_address:#x}")
            }
            TinyAllocatorError::GenerationMismatch {
                vm_address,
                expected_gen,
                actual_gen,
            } => {
                write!(
                    f,
                    "generation mismatch at vm_address {vm_address:#x}: \
                     expected gen {expected_gen}, found gen {actual_gen}"
                )
            }
            TinyAllocatorError::DoubleFree { vm_address } => {
                write!(f, "double-free detected at vm_address {vm_address:#x}")
            }
            TinyAllocatorError::NativeAllocFailed => {
                write!(f, "native allocator (Ralloc) returned an error")
            }
            TinyAllocatorError::ShutdownInProgress => {
                write!(f, "allocator is shut down; no new allocations are accepted")
            }
        }
    }
}

impl std::error::Error for TinyAllocatorError {}

// ── AllocationResult ──────────────────────────────────────────────────────────

/// Describes a successful allocation produced by [`TinyAllocator::allocate`].
#[derive(Debug)]
pub struct AllocationResult {
    /// VM-side address of the newly allocated slot.
    pub vm_address:     usize,
    /// Generation counter of the slot at the time of allocation.
    pub vm_generation:  u64,
    /// Native allocator id for this allocation.
    // PHASE3: replace with VmAllocation
    pub native_id: u64,
    /// The byte size that was actually recorded (equal to `size` in Phase 2).
    pub effective_size: usize,
}

// ── TinyAllocatorConfig ───────────────────────────────────────────────────────

/// Configuration for a [`TinyAllocator`] instance.
#[derive(Debug, Clone)]
pub struct TinyAllocatorConfig {
    /// Capacity of the [`MemoryLog`] ring buffer (number of entries).
    ///
    /// Default: 1 024.
    pub log_capacity:          usize,
    /// Capacity of the [`MemoryErrorPusher`] queue when one is created
    /// automatically.  Not directly used by [`TinyAllocator`] itself, but
    /// exposed here so callers can read it when constructing their own pusher.
    ///
    /// Default: 256.
    pub error_pusher_capacity: usize,
    /// Whether the [`MemoryLog`] starts enabled.
    ///
    /// Default: `true`.
    pub enable_logging:        bool,
}

impl Default for TinyAllocatorConfig {
    fn default() -> Self {
        Self {
            log_capacity:          1024,
            error_pusher_capacity: 256,
            enable_logging:        true,
        }
    }
}

// ── ShutdownReport ────────────────────────────────────────────────────────────

/// Summary returned by [`TinyAllocator::shutdown_drain`].
#[derive(Debug, Clone)]
pub struct ShutdownReport {
    /// Number of live allocations that were present at shutdown.
    pub live_count:      usize,
    /// Total live bytes present at shutdown.
    pub live_bytes:      usize,
    /// Cumulative number of allocations made over the lifetime of this
    /// allocator instance.
    pub total_allocated: u64,
    /// Cumulative number of successful frees over the lifetime of this
    /// allocator instance.
    pub total_freed:     u64,
}

// ── TinyAllocator ─────────────────────────────────────────────────────────────

/// The boundary layer between TinyOne's VM heap and native (Ralloc) memory.
///
/// `TinyAllocator` is the Phase 2 stub; it records, logs, and hooks every
/// allocation operation but delegates all native memory management to
/// placeholder logic.  Phase 3 will replace those stubs with real Ralloc calls.
///
/// All methods take `&self` (shared reference); interior mutability is provided
/// by the locks inside each sub-component.
///
/// # Shutdown
/// Once [`shutdown_drain`] is called the `shutdown` flag is set to `true` and
/// subsequent calls to [`allocate`] return
/// [`TinyAllocatorError::ShutdownInProgress`].  [`free`] and [`reallocate`]
/// still attempt to operate on existing records so that cleanup can complete.
///
/// [`shutdown_drain`]: TinyAllocator::shutdown_drain
/// [`allocate`]: TinyAllocator::allocate
/// [`free`]: TinyAllocator::free
/// [`reallocate`]: TinyAllocator::reallocate
pub struct TinyAllocator {
    table:    AllocTable,
    log:      MemoryLog,
    hooks:    HookRegistry,
    /// Monotonic sequence counter; used as both log `seq` and as the
    /// placeholder native allocation id.
    seq:      AtomicU64,
    /// Set to `true` by [`shutdown_drain`] to block further allocations.
    ///
    /// [`shutdown_drain`]: TinyAllocator::shutdown_drain
    shutdown: AtomicBool,
}

impl TinyAllocator {
    // ── Construction ──────────────────────────────────────────────────────────

    /// Create a new allocator with the given [`TinyAllocatorConfig`].
    pub fn new(config: TinyAllocatorConfig) -> Self {
        let log = MemoryLog::new(config.log_capacity);
        if !config.enable_logging {
            log.disable();
        }
        Self {
            table: AllocTable::new(),
            log,
            hooks: HookRegistry::new(),
            seq: AtomicU64::new(1),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Create a new allocator with [`TinyAllocatorConfig::default`] settings.
    pub fn with_defaults() -> Self {
        Self::new(TinyAllocatorConfig::default())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Atomically fetch-and-increment the sequence counter.
    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Allocate `size` bytes for the VM heap slot at `vm_address` / `vm_generation`.
    ///
    /// Steps (Phase 2):
    /// 1. Reject if the shutdown flag is set.
    /// 2. Reject zero-size allocations (Phase 3 may relax this for ZSTs).
    /// 3. Insert an [`AllocRecord`] into the [`AllocTable`].
    /// 4. Log a success entry.
    /// 5. Dispatch [`MemoryEvent::Allocated`].
    ///
    /// # Errors
    /// Returns [`TinyAllocatorError::ShutdownInProgress`] if the allocator has
    /// been shut down, [`TinyAllocatorError::InvalidSize`] for zero sizes, or
    /// [`TinyAllocatorError::AllocationTableFull`] if the table already holds a
    /// live record at `vm_address`.
    pub fn allocate(
        &self,
        vm_address: usize,
        vm_generation: u64,
        kind: AllocKind,
        size: usize,
        thread_id: u64,
    ) -> Result<AllocationResult, TinyAllocatorError> {
        // 1. Shutdown guard.
        if self.shutdown.load(Ordering::Acquire) {
            return Err(TinyAllocatorError::ShutdownInProgress);
        }

        // 2. Size validation.
        if size == 0 {
            return Err(TinyAllocatorError::InvalidSize { size });
        }

        // 3. Claim the next sequence number; this doubles as the placeholder
        //    native allocation id for Phase 2.
        let seq = self.next_seq();

        // PHASE3: replace with VmAllocation obtained from VmAllocator::global().allocate(size, align)
        let native_id: u64 = seq; // PHASE3: replace with VmAllocation

        // 4. Record in the allocation table.
        let record = AllocRecord {
            vm_address,
            vm_generation,
            native_handle: Some(VmAllocHandle(native_id)),
            kind,
            byte_len: size,
            capacity: size,
            arena_id: 0,
            log_seq: seq,
            live: true,
        };
        self.table.insert(record).map_err(|e| {
            match e {
                AllocTableError::AlreadyExists => TinyAllocatorError::AllocationTableFull,
                // Other errors cannot occur on insert.
                _ => TinyAllocatorError::AllocationTableFull,
            }
        })?;

        // 5. Log success.
        self.log
            .log(MemoryLogEntry::success(seq, thread_id, OperationType::Allocate, vm_address, vm_generation, size));

        // 6. Dispatch hook event.
        self.hooks.dispatch(MemoryEvent::Allocated {
            vm_address,
            vm_generation,
            size,
            type_name: kind.type_name(),
        });

        Ok(AllocationResult {
            vm_address,
            vm_generation,
            native_id,
            effective_size: size,
        })
    }

    /// Free the allocation at `vm_address` with `vm_generation`.
    ///
    /// Generation is validated before removal.  A mismatch triggers a
    /// [`MemoryEvent::StalePointer`]; an address that was never allocated (or
    /// already freed) triggers [`MemoryEvent::DoubleFree`].
    ///
    /// # Errors
    /// Returns [`TinyAllocatorError::GenerationMismatch`] or
    /// [`TinyAllocatorError::DoubleFree`] on failure.
    pub fn free(&self, vm_address: usize, vm_generation: u64, thread_id: u64) -> Result<(), TinyAllocatorError> {
        let seq = self.next_seq();

        match self.table.remove(vm_address, vm_generation) {
            Ok(_record) => {
                // PHASE3: call VmAllocator::global().deallocate(record.native_handle_as_vmallocation())

                self.log.log(MemoryLogEntry::success(
                    seq,
                    thread_id,
                    OperationType::Free,
                    vm_address,
                    vm_generation,
                    0,
                ));

                self.hooks.dispatch(MemoryEvent::Freed {
                    vm_address,
                    vm_generation,
                });

                Ok(())
            }

            Err(AllocTableError::GenerationMismatch { expected, actual }) => {
                self.log.log(MemoryLogEntry::failure(
                    seq,
                    thread_id,
                    OperationType::Error,
                    vm_address,
                    vm_generation,
                    "generation_mismatch",
                ));

                self.hooks.dispatch(MemoryEvent::StalePointer {
                    vm_address,
                    expected_gen: expected,
                    actual_gen: actual,
                });

                Err(TinyAllocatorError::GenerationMismatch {
                    vm_address,
                    expected_gen: expected,
                    actual_gen: actual,
                })
            }

            Err(AllocTableError::NotFound) | Err(AllocTableError::AlreadyDead) => {
                self.log.log(MemoryLogEntry::failure(
                    seq,
                    thread_id,
                    OperationType::Error,
                    vm_address,
                    vm_generation,
                    "double_free",
                ));

                self.hooks.dispatch(MemoryEvent::DoubleFree {
                    vm_address,
                    vm_generation,
                });

                Err(TinyAllocatorError::DoubleFree { vm_address })
            }

            Err(AllocTableError::AlreadyExists) => {
                // Cannot happen on remove, but handle exhaustively.
                Err(TinyAllocatorError::DoubleFree { vm_address })
            }
        }
    }

    /// Resize the allocation at `vm_address` / `vm_generation` to `new_size`.
    ///
    /// Phase 2 updates the size recorded in the table.  Phase 3 will issue a
    /// real `VmAllocator::reallocate` call and update the native handle.
    ///
    /// # Errors
    /// Returns [`TinyAllocatorError::NotFound`] or
    /// [`TinyAllocatorError::GenerationMismatch`] if the address/generation pair
    /// is not a live allocation, or [`TinyAllocatorError::InvalidSize`] for
    /// a zero `new_size`.
    pub fn reallocate(
        &self,
        vm_address: usize,
        vm_generation: u64,
        new_size: usize,
        thread_id: u64,
    ) -> Result<(), TinyAllocatorError> {
        let seq = self.next_seq();

        if new_size == 0 {
            return Err(TinyAllocatorError::InvalidSize { size: new_size });
        }

        // Look up the current record to get old_size for the hook event.
        let old_record = self.table.get(vm_address, vm_generation).ok_or_else(|| {
            // Distinguish NotFound from GenerationMismatch via a targeted remove
            // probe — cheaper than duplicating lookup logic.
            match self.table.remove(vm_address, vm_generation) {
                Err(AllocTableError::GenerationMismatch { expected, actual }) => {
                    TinyAllocatorError::GenerationMismatch {
                        vm_address,
                        expected_gen: expected,
                        actual_gen: actual,
                    }
                }
                _ => TinyAllocatorError::NotFound { vm_address },
            }
        })?;

        let old_size = old_record.byte_len;

        // PHASE3: VmAllocator::global().reallocate(&mut native_handle, new_size)?

        // Phase 2: update by remove + re-insert with the new size.
        // We already hold a `get` snapshot so remove cannot fail with a
        // different error.
        let removed = self
            .table
            .remove(vm_address, vm_generation)
            .map_err(|_| TinyAllocatorError::NotFound { vm_address })?;

        let updated = AllocRecord {
            byte_len: new_size,
            capacity: new_size,
            log_seq: seq,
            ..removed
        };
        self.table
            .insert(updated)
            .map_err(|_| TinyAllocatorError::AllocationTableFull)?;

        self.log.log(MemoryLogEntry::success(
            seq,
            thread_id,
            OperationType::Realloc,
            vm_address,
            vm_generation,
            new_size,
        ));

        self.hooks.dispatch(MemoryEvent::Reallocated {
            vm_address,
            vm_generation,
            old_size,
            new_size,
        });

        Ok(())
    }

    /// Validate that `vm_address` / `vm_generation` is a live allocation.
    ///
    /// This is a read-only check; it does not modify the table.  It logs the
    /// attempt and returns `Ok(())` if the record is found, or an error if the
    /// address is unknown or the generation is stale.
    ///
    /// # Errors
    /// Returns [`TinyAllocatorError::GenerationMismatch`] if the address exists
    /// but the generation is wrong, or [`TinyAllocatorError::NotFound`] if the
    /// address is unknown.
    pub fn validate(
        &self,
        vm_address: usize,
        vm_generation: u64,
        operation: &'static str,
        thread_id: u64,
    ) -> Result<(), TinyAllocatorError> {
        // Phase 2: `operation` is reserved for enriched log entries in Phase 3.
        let _ = operation;
        let seq = self.next_seq();

        match self.table.get(vm_address, vm_generation) {
            Some(_record) => {
                self.log.log(MemoryLogEntry::success(
                    seq,
                    thread_id,
                    OperationType::Validate,
                    vm_address,
                    vm_generation,
                    0,
                ));
                Ok(())
            }
            None => {
                // To differentiate NotFound from GenerationMismatch we need to
                // probe without a generation filter.  Use a raw remove-probe
                // pattern without actually mutating the table: peek via a
                // mismatched generation to see if the address exists at all.
                // Since `table.get` returns None for both cases, we do a
                // secondary generationless check by attempting to get with
                // generation 0 (which `AllocTable::get` treats as "any").
                // However, the AllocTable implementation compares exactly, so
                // generation 0 is treated as the literal generation 0.
                //
                // Safest approach: use the same pattern as free() — treat
                // ambiguous None as GenerationMismatch (stale pointer), which
                // is the correct safety response in either case.
                self.log.log(MemoryLogEntry::failure(
                    seq,
                    thread_id,
                    OperationType::Error,
                    vm_address,
                    vm_generation,
                    "stale_pointer",
                ));

                self.hooks.dispatch(MemoryEvent::StalePointer {
                    vm_address,
                    expected_gen: vm_generation,
                    actual_gen: 0, // unknown; we can't recover it without exposing more table API
                });

                // Return GenerationMismatch as the canonical error for any
                // failed validate — Phase 3 can refine this with table introspection.
                Err(TinyAllocatorError::GenerationMismatch {
                    vm_address,
                    expected_gen: vm_generation,
                    actual_gen: 0,
                })
            }
        }
    }

    /// Report a memory access violation without returning an error.
    ///
    /// This is a side-effect-only method intended for cases where the VM has
    /// already decided the access is illegal and needs to record that fact for
    /// diagnostics.  It logs an [`OperationType::Error`] entry and dispatches
    /// an [`MemoryEvent::AccessViolation`].
    pub fn report_access_violation(
        &self,
        vm_address: usize,
        vm_generation: u64,
        operation: &'static str,
        reason: &'static str,
        thread_id: u64,
    ) {
        let seq = self.next_seq();

        self.log.log(MemoryLogEntry::failure(
            seq,
            thread_id,
            OperationType::Error,
            vm_address,
            vm_generation,
            "access_violation",
        ));

        self.hooks.dispatch(MemoryEvent::AccessViolation {
            vm_address,
            vm_generation,
            operation,
            reason,
        });
    }

    /// Drain all live allocations for orderly shutdown.
    ///
    /// After this returns, the `shutdown` flag is set and no further
    /// allocations will be accepted.  The method logs a
    /// [`OperationType::Shutdown`] entry and dispatches
    /// [`MemoryEvent::ShutdownDrain`].
    ///
    /// Returns a [`ShutdownReport`] describing what was live at shutdown.
    pub fn shutdown_drain(&self, thread_id: u64) -> ShutdownReport {
        // Capture stats before we drain (drain clears the table but not the
        // lifetime counters).
        let pre_stats = self.table.stats();

        // Set shutdown flag before draining so concurrent allocate() calls that
        // check the flag after this point see shutdown=true. SeqCst ensures the
        // store is globally visible before the drain proceeds.
        self.shutdown.store(true, Ordering::SeqCst);

        let live_records = self.table.drain_for_shutdown();
        let live_count = live_records.iter().filter(|r| r.live).count();
        let live_bytes: usize = live_records.iter().filter(|r| r.live).map(|r| r.byte_len).sum();

        // PHASE3: for each live record, call VmAllocator::global().deallocate(...)

        let seq = self.next_seq();
        self.log
            .log(MemoryLogEntry::success(seq, thread_id, OperationType::Shutdown, 0, 0, 0));

        self.hooks
            .dispatch(MemoryEvent::ShutdownDrain { live_count, live_bytes });

        ShutdownReport {
            live_count,
            live_bytes,
            total_allocated: pre_stats.total_allocated,
            total_freed: pre_stats.total_freed,
        }
    }

    /// Register a memory event hook.
    ///
    /// Hooks are invoked in registration order whenever a memory event occurs.
    /// A panicking hook is caught and printed to stderr; it does not affect
    /// other hooks or crash the allocator.
    pub fn register_hook(&self, hook: Arc<dyn VmMemoryHook>) {
        self.hooks.register(hook);
    }

    /// Return a snapshot of all entries currently in the log (oldest first).
    ///
    /// Returns an empty `Vec` if the log is disabled or its internal lock is
    /// poisoned.
    pub fn log_snapshot(&self) -> Vec<MemoryLogEntry> {
        self.log.snapshot()
    }

    /// Return aggregate statistics from the underlying [`AllocTable`].
    pub fn stats(&self) -> AllocTableStats {
        self.table.stats()
    }

    /// Enable or disable the [`MemoryLog`].
    ///
    /// When disabled, all log calls are no-ops.  Hook dispatch is unaffected.
    pub fn set_logging_enabled(&self, enabled: bool) {
        if enabled {
            self.log.enable();
        } else {
            self.log.disable();
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm_hooks::MemoryErrorPusher;

    fn allocator() -> TinyAllocator {
        TinyAllocator::with_defaults()
    }

    // 1. allocate_and_free — allocate, then free, verify stats ─────────────────

    #[test]
    fn allocate_and_free() {
        let a = allocator();
        let result = a.allocate(1, 1, AllocKind::String, 64, 0).unwrap();

        assert_eq!(result.vm_address, 1);
        assert_eq!(result.vm_generation, 1);
        assert_eq!(result.effective_size, 64);

        let stats = a.stats();
        assert_eq!(stats.live_count, 1);
        assert_eq!(stats.live_bytes, 64);
        assert_eq!(stats.total_allocated, 1);
        assert_eq!(stats.total_freed, 0);

        a.free(1, 1, 0).unwrap();

        let stats = a.stats();
        assert_eq!(stats.live_count, 0);
        assert_eq!(stats.live_bytes, 0);
        assert_eq!(stats.total_freed, 1);
    }

    // 2. free_wrong_generation_returns_error ───────────────────────────────────

    #[test]
    fn free_wrong_generation_returns_error() {
        let a = allocator();
        let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
        a.register_hook(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);

        a.allocate(2, 1, AllocKind::Array, 128, 0).unwrap();

        // Free with generation 99 instead of 1.
        let err = a.free(2, 99, 0).unwrap_err();
        assert!(
            matches!(
                err,
                TinyAllocatorError::GenerationMismatch {
                    vm_address: 2,
                    expected_gen: 99,
                    ..
                }
            ),
            "expected GenerationMismatch, got {err:?}"
        );
        assert!(err.is_safety_violation(), "GenerationMismatch must be a safety violation");

        // Hook should have received a StalePointer event.
        assert!(pusher.has_errors(), "expected at least one error event in pusher");
        let events = pusher.drain_errors();
        assert!(
            events.iter().any(|e| matches!(e, MemoryEvent::StalePointer { .. })),
            "expected StalePointer event; got {events:?}"
        );
    }

    // 3. double_free_returns_error ─────────────────────────────────────────────

    #[test]
    fn double_free_returns_error() {
        let a = allocator();
        a.allocate(3, 1, AllocKind::Buffer, 32, 0).unwrap();
        a.free(3, 1, 0).unwrap();

        let err = a.free(3, 1, 0).unwrap_err();
        assert!(matches!(err, TinyAllocatorError::DoubleFree { vm_address: 3 }), "expected DoubleFree, got {err:?}");
        assert!(err.is_safety_violation(), "DoubleFree must be a safety violation");
    }

    // 4. shutdown_drain_empties_table ──────────────────────────────────────────

    #[test]
    fn shutdown_drain_empties_table() {
        let a = allocator();
        a.allocate(10, 1, AllocKind::String, 8, 0).unwrap();
        a.allocate(11, 1, AllocKind::Array, 16, 0).unwrap();
        a.allocate(12, 1, AllocKind::Map, 24, 0).unwrap();

        let report = a.shutdown_drain(0);

        assert_eq!(report.live_count, 3, "all three should be reported as live");
        assert_eq!(report.live_bytes, 8 + 16 + 24);
        assert_eq!(report.total_allocated, 3);
        assert_eq!(report.total_freed, 0);

        // Table must be empty.
        let stats = a.stats();
        assert_eq!(stats.live_count, 0);

        // Further allocations must fail.
        let err = a.allocate(20, 1, AllocKind::String, 4, 0).unwrap_err();
        assert!(matches!(err, TinyAllocatorError::ShutdownInProgress));
    }

    // 5. hook_receives_allocated_event — Allocated is not an error event ───────

    #[test]
    fn hook_receives_allocated_event() {
        let a = allocator();
        let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
        a.register_hook(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);

        a.allocate(5, 1, AllocKind::Vec, 64, 0).unwrap();

        // Allocated is NOT an error event, so the pusher should be empty.
        assert!(!pusher.has_errors(), "Allocated event should not be enqueued as an error");
    }

    // 6. hook_receives_stale_pointer_on_bad_free ───────────────────────────────

    #[test]
    fn hook_receives_stale_pointer_on_bad_free() {
        let a = allocator();
        let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
        a.register_hook(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);

        a.allocate(6, 1, AllocKind::Closure, 16, 0).unwrap();
        let _ = a.free(6, 999, 0); // wrong generation

        assert!(pusher.has_errors(), "a StalePointer event should have been pushed");
    }

    // 7. log_captures_operations ───────────────────────────────────────────────

    #[test]
    fn log_captures_operations() {
        let a = allocator();
        a.allocate(7, 1, AllocKind::Struct, 200, 0).unwrap();
        a.free(7, 1, 0).unwrap();

        let snap = a.log_snapshot();
        // At minimum 2 entries: one Allocate, one Free.
        assert!(snap.len() >= 2, "expected at least 2 log entries, got {}", snap.len());

        let has_alloc = snap.iter().any(|e| e.op_type == OperationType::Allocate);
        let has_free = snap.iter().any(|e| e.op_type == OperationType::Free);
        assert!(has_alloc, "log must contain an Allocate entry");
        assert!(has_free, "log must contain a Free entry");
    }
}
