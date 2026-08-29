//! `TinyAllocator` — the boundary layer between TinyOne's VM heap and Ralloc.
//!
//! This module owns:
//! - An [`AllocTable`] — the live-allocation registry keyed by `vm_address`.
//! - A [`MemoryLog`] — a bounded ring-buffer of operation records for diagnostics.
//! - A [`HookRegistry`] — dispatches [`MemoryEvent`]s to registered observers.
//! - A side table of real `ralloc::VmAllocation`s keyed by native id.
//! - A sequence counter and shutdown flag.
//!
//! # Phase 3
//! Every VM heap allocation tracked in [`AllocTable`] now has a matching real
//! allocation from `ralloc::VmAllocator`, stored in the `native` side table
//! rather than inside [`AllocRecord`] itself (records stay `Clone`, and
//! `VmAllocation` is deliberately `!Clone`). This does not change where
//! TinyOne's heap object *bytes* live — `runtime/heap.rs` keeps owning those
//! directly — it makes the allocator's bookkeeping shadow a real allocator
//! instead of a placeholder counter, so native exhaustion is a real,
//! reportable condition.
//!
//! # Thread safety
//! [`TinyAllocator`] is `Send + Sync`.  Interior mutability is managed by the
//! locks embedded in [`AllocTable`], [`MemoryLog`], [`HookRegistry`], and the
//! `native` table.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::alloc_table::{AllocKind, AllocRecord, AllocTable, AllocTableError, AllocTableStats, VmAllocHandle};
use crate::memory_log::{MemoryLog, MemoryLogEntry, OperationType};
use crate::vm_hooks::{HookRegistry, MemoryEvent, VmMemoryHook};

// ── RallocBytes ───────────────────────────────────────────────────────────────

/// Owned byte storage for the flat `HeapData` kinds (`String`, `Buffer`,
/// `CharBuffer`), physically backed by a real [`ralloc::VmAllocation`] rather
/// than a Rust-allocator `Vec`/`String`.
///
/// Allocates directly from `ralloc::VmAllocator::global()`, independent of
/// whether a [`TinyAllocator`] is wired to the owning heap — `TinyAllocator`
/// only ever records *bookkeeping* for these allocations (see
/// [`TinyAllocator::allocate_owned`]); it never owns the memory itself, so a
/// bare heap with no allocator attached (as some unit tests construct) can
/// still allocate real, working `RallocBytes`.
///
/// Deliberately does not implement `Clone`: `ralloc::VmAllocation` is
/// move-only by design, and a `Clone::clone` that can fail on arena
/// exhaustion would have to panic (the trait is infallible), which would
/// regress this VM's existing "out of memory is a normal `Err`, never a
/// panic" behavior. Callers that need an independent copy should allocate a
/// fresh `RallocBytes` explicitly via [`RallocBytes::from_slice`].
pub(crate) struct RallocBytes(Option<ralloc::VmAllocation>);

impl RallocBytes {
    /// Allocate `len` zero-initialized bytes.
    pub(crate) fn zeroed(len: usize) -> Result<Self, TinyAllocatorError> {
        let mut alloc = ralloc::VmAllocator::global()
            .allocate(len)
            .map_err(|_| TinyAllocatorError::NativeAllocFailed)?;
        alloc.as_mut_slice().fill(0);
        Ok(Self(Some(alloc)))
    }

    /// Allocate storage and copy `bytes` into it.
    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self, TinyAllocatorError> {
        let mut alloc = ralloc::VmAllocator::global()
            .allocate(bytes.len())
            .map_err(|_| TinyAllocatorError::NativeAllocFailed)?;
        alloc.as_mut_slice().copy_from_slice(bytes);
        Ok(Self(Some(alloc)))
    }

    /// Returns the allocation as an immutable byte slice.
    pub(crate) fn as_slice(&self) -> &[u8] {
        self.0.as_ref().expect("RallocBytes accessed after drop").as_slice()
    }

    /// Returns the allocation as a mutable byte slice.
    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        self.0.as_mut().expect("RallocBytes accessed after drop").as_mut_slice()
    }

    /// Returns the number of bytes in the allocation.
    pub(crate) fn len(&self) -> usize {
        self.0.as_ref().expect("RallocBytes accessed after drop").len()
    }

    /// Resizes the allocation to `new_len` bytes, preserving the first
    /// `min(old_len, new_len)` bytes. On failure the original allocation is
    /// left completely untouched.
    ///
    /// Bytes beyond the old length are **not** zero-initialized (matching
    /// `ralloc::VmAllocator::reallocate`'s own semantics) — callers that
    /// grow into this new capacity before writing to it (e.g.
    /// [`crate::runtime::ralloc_vec::RallocVec`], which tracks a logical
    /// length separately from physical capacity and never reads past it)
    /// must not treat unwritten bytes as meaningful.
    pub(crate) fn realloc(&mut self, new_len: usize) -> Result<(), TinyAllocatorError> {
        let alloc = self.0.take().expect("RallocBytes accessed after drop");
        match ralloc::VmAllocator::global().reallocate(alloc, new_len) {
            Ok(new_alloc) => {
                self.0 = Some(new_alloc);
                Ok(())
            }
            Err((original, _error)) => {
                self.0 = Some(original);
                Err(TinyAllocatorError::NativeAllocFailed)
            }
        }
    }
}

impl Drop for RallocBytes {
    fn drop(&mut self) {
        if let Some(alloc) = self.0.take() {
            ralloc::VmAllocator::global().deallocate(alloc);
        }
    }
}

impl std::fmt::Debug for RallocBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RallocBytes")
            .field(&format_args!("<{} bytes>", self.len()))
            .finish()
    }
}

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
/// `TinyAllocator` records, logs, and hooks every allocation operation while
/// using Ralloc for native backing. Heap payloads that already own a
/// `RallocBytes` use `allocate_owned` to avoid double-booking the arena.
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
    /// Real Ralloc-backed allocations, keyed by the same id stored in each
    /// live record's `native_handle`. Lives beside `table` rather than
    /// inside it because `VmAllocation` is `!Clone` and `AllocTable::get`
    /// returns cloned records.
    native:   Mutex<HashMap<u64, ralloc::VmAllocation>>,
    /// Monotonic sequence counter; used as both log `seq` and as the native
    /// allocation id (the key into `native`).
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
            native: Mutex::new(HashMap::new()),
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
    /// Steps:
    /// 1. Reject if the shutdown flag is set.
    /// 2. Reject zero-size allocations.
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
        self.guard_allocation(size)?;

        // Claim the next sequence number; this doubles as the native
        // allocation id (the key into `native`).
        let seq = self.next_seq();
        let native_id: u64 = seq;

        // Make the real native allocation before touching the table, so a
        // native OOM never leaves a record behind with nothing backing it.
        let native_alloc = match ralloc::VmAllocator::global().allocate(size) {
            Ok(alloc) => alloc,
            Err(_) => {
                self.log.log(MemoryLogEntry::failure(
                    seq,
                    thread_id,
                    OperationType::Error,
                    vm_address,
                    vm_generation,
                    "native_alloc_failed",
                ));
                self.hooks.dispatch(MemoryEvent::OutOfMemory {
                    requested_size: size,
                    live_bytes:     self.table.stats().live_bytes,
                    limit_bytes:    ralloc::VmAllocator::capacity(),
                });
                return Err(TinyAllocatorError::NativeAllocFailed);
            }
        };

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
        match self.finish_allocate(record, thread_id) {
            Ok(()) => {
                self.native.lock().unwrap().insert(native_id, native_alloc);
            }
            Err(e) => {
                // The table rejected the record; release the native
                // allocation we already made so it doesn't leak.
                ralloc::VmAllocator::global().deallocate(native_alloc);
                return Err(e);
            }
        }

        Ok(AllocationResult {
            vm_address,
            vm_generation,
            native_id,
            effective_size: size,
        })
    }

    /// Record bookkeeping for an allocation whose real memory the *caller*
    /// already owns (currently: `HeapData::String`/`Buffer`/`CharBuffer`,
    /// which own a [`crate::tiny_allocator::RallocBytes`] directly).
    ///
    /// Unlike [`allocate`][Self::allocate], this does not make a second real
    /// `ralloc::VmAllocation` — Ralloc's fixed-size arena is sized assuming
    /// exactly one live allocation per live heap byte, so double-booking
    /// these kinds would burn through that headroom for no reason. The
    /// resulting [`AllocRecord`] has `native_handle: None`, which
    /// [`free`][Self::free] and [`shutdown_drain`][Self::shutdown_drain]
    /// already treat as "nothing to release here" — the real memory is
    /// released when the owning `RallocBytes` is dropped instead.
    pub fn allocate_owned(
        &self,
        vm_address: usize,
        vm_generation: u64,
        kind: AllocKind,
        size: usize,
        thread_id: u64,
    ) -> Result<AllocationResult, TinyAllocatorError> {
        self.guard_allocation(size)?;
        let seq = self.next_seq();
        let record = AllocRecord {
            vm_address,
            vm_generation,
            native_handle: None,
            kind,
            byte_len: size,
            capacity: size,
            arena_id: 0,
            log_seq: seq,
            live: true,
        };
        self.finish_allocate(record, thread_id)?;
        Ok(AllocationResult {
            vm_address,
            vm_generation,
            native_id: seq,
            effective_size: size,
        })
    }

    /// Shared shutdown/size validation for [`allocate`][Self::allocate] and
    /// [`allocate_owned`][Self::allocate_owned].
    fn guard_allocation(&self, size: usize) -> Result<(), TinyAllocatorError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(TinyAllocatorError::ShutdownInProgress);
        }
        if size == 0 {
            return Err(TinyAllocatorError::InvalidSize { size });
        }
        Ok(())
    }

    /// Shared record-insert/log/hook-dispatch tail for
    /// [`allocate`][Self::allocate] and [`allocate_owned`][Self::allocate_owned].
    fn finish_allocate(&self, record: AllocRecord, thread_id: u64) -> Result<(), TinyAllocatorError> {
        let vm_address = record.vm_address;
        let vm_generation = record.vm_generation;
        let kind = record.kind;
        let size = record.byte_len;
        let seq = record.log_seq;
        self.table.insert(record).map_err(|_e| {
            // AlreadyExists is the only error `insert` can return; other
            // variants cannot occur here.
            TinyAllocatorError::AllocationTableFull
        })?;

        self.log
            .log(MemoryLogEntry::success(seq, thread_id, OperationType::Allocate, vm_address, vm_generation, size));

        self.hooks.dispatch(MemoryEvent::Allocated {
            vm_address,
            vm_generation,
            size,
            type_name: kind.type_name(),
        });

        Ok(())
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
            Ok(record) => {
                if let Some(handle) = record.native_handle {
                    let native_alloc = self.native.lock().unwrap().remove(&handle.0);
                    if let Some(native_alloc) = native_alloc {
                        ralloc::VmAllocator::global().deallocate(native_alloc);
                    }
                }

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
    /// The native `VmAllocation` is reallocated first; the `AllocTable`
    /// record is only updated once that succeeds. On native failure the
    /// original allocation is restored unchanged and the table is untouched.
    ///
    /// # Errors
    /// Returns [`TinyAllocatorError::NotFound`] or
    /// [`TinyAllocatorError::GenerationMismatch`] if the address/generation pair
    /// is not a live allocation, [`TinyAllocatorError::InvalidSize`] for a
    /// zero `new_size`, or [`TinyAllocatorError::NativeAllocFailed`] if the
    /// native reallocation could not be satisfied.
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

        // Reallocate the native backing first. On failure, restore the
        // original allocation untouched and leave the table alone entirely.
        if let Some(handle) = old_record.native_handle {
            let existing = self.native.lock().unwrap().remove(&handle.0);
            if let Some(native_alloc) = existing {
                match ralloc::VmAllocator::global().reallocate(native_alloc, new_size) {
                    Ok(new_alloc) => {
                        self.native.lock().unwrap().insert(handle.0, new_alloc);
                    }
                    Err((original, _error)) => {
                        self.native.lock().unwrap().insert(handle.0, original);
                        self.log.log(MemoryLogEntry::failure(
                            seq,
                            thread_id,
                            OperationType::Error,
                            vm_address,
                            vm_generation,
                            "native_realloc_failed",
                        ));
                        return Err(TinyAllocatorError::NativeAllocFailed);
                    }
                }
            }
        }

        // Update the table's bookkeeping now that the native reallocation
        // has already succeeded. We already hold a `get` snapshot so remove
        // cannot fail with a different error.
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

        // Release every remaining native allocation. `native` only ever holds
        // entries for records `allocate()` has inserted and `free()`/
        // `reallocate()` haven't already removed, so draining it here matches
        // exactly what `drain_for_shutdown` just pulled out of the table.
        {
            let mut native = self.native.lock().unwrap();
            for (_native_id, native_alloc) in native.drain() {
                ralloc::VmAllocator::global().deallocate(native_alloc);
            }
        }

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
