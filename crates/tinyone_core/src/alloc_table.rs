//! Internal allocation table: bridges TinyOne's generation-tagged heap slots
//! to native (Ralloc) allocation handles.
//!
//! Every live VM heap slot has at most one [`AllocRecord`] here, keyed by
//! `vm_address`. The generation field mirrors `TinyHeap::generations` so that
//! stale [`HeapRef`]s are rejected without consulting the heap itself.
//!
//! Phase 2 will replace [`VmAllocHandle`] with the real Ralloc handle type and
//! wire the table's `insert`/`remove` calls to actual allocator operations.

use std::collections::HashMap;
use std::sync::Mutex;

// ── Handle ────────────────────────────────────────────────────────────────────

/// Opaque placeholder for a Ralloc native allocation handle.
///
/// Will be replaced with the actual Ralloc type in Phase 2. The inner `u64`
/// is treated as an opaque token; callers must not construct or interpret it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VmAllocHandle(pub u64);

// ── AllocKind ─────────────────────────────────────────────────────────────────

/// The kind of TinyOne heap object backed by this allocation.
///
/// Mirrors the set of [`HeapData`][crate::HeapData] variants so the table can
/// be inspected without holding a reference to the heap itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocKind {
    /// A UTF-8 string object (`HeapData::String`).
    String,
    /// A dynamic array of `Value`s (`HeapData::Array`).
    Array,
    /// A raw byte buffer (`HeapData::Buffer`).
    Buffer,
    /// A named-field struct (`HeapData::Struct`).
    Struct,
    /// A mutable cell wrapping a single `Value` (`HeapData::Cell`).
    Cell,
    /// A key-value map (`HeapData::Map`).
    Map,
    /// A VM-level mutex (`HeapData::Mutex`).
    Mutex,
    /// An atomic 64-bit integer (`HeapData::Atomic`).
    Atomic,
    /// A spawned OS thread handle (`HeapData::Thread`).
    Thread,
    /// A Unicode scalar value (`HeapData::Char`).
    Char,
    /// A buffer of Unicode scalar values (`HeapData::CharBuffer`).
    CharBuffer,
    /// A resizable sequence of `Value`s (`HeapData::Vec`).
    Vec,
    /// A record (anonymous struct) (`HeapData::Record`).
    Record,
    /// An associative dictionary (`HeapData::Dictionary`).
    Dictionary,
    /// An owned, heap-allocated `Value` box (`HeapData::Box`).
    Box,
    /// A raw typed allocation (`HeapData::Alloc`).
    Raw,
    /// A captured closure (`HeapData::Closure`).
    Closure,
    /// A sum type with an optional payload (`HeapData::Sum`).
    Sum,
    /// A plain enum variant (`HeapData::Enum`).
    Enum,
    /// A tagged union with a mandatory payload (`HeapData::TaggedUnion`).
    TaggedUnion,
    /// An `Ok`/`Err` result wrapper (`HeapData::Result`).
    Result,
    /// A `Some`/`None` optional wrapper (`HeapData::Option`).
    Option,
    /// A dynamically-dispatched trait object (`HeapData::Dyn`).
    Dyn,
    /// An OS file descriptor (`HeapData::FileDescriptor`).
    FileDescriptor,
}

// ── AllocRecord ───────────────────────────────────────────────────────────────

/// A record for a single VM heap slot's native allocation.
///
/// The `vm_address` + `vm_generation` pair uniquely identifies the *current
/// occupant* of a slot — matching the semantics of [`HeapRef`][crate::HeapRef].
#[derive(Debug, Clone)]
pub struct AllocRecord {
    /// Index into the VM's heap object array (`HeapRef::address`).
    pub vm_address:    usize,
    /// Generation counter at the time of allocation (`HeapRef::generation`).
    pub vm_generation: u64,
    /// Native allocator handle, or `None` if not yet backed by Ralloc.
    pub native_handle: Option<VmAllocHandle>,
    /// The kind of heap object stored in this slot.
    pub kind:          AllocKind,
    /// Logical byte length of the object at allocation time.
    pub byte_len:      usize,
    /// Allocated capacity (may exceed `byte_len` for growable objects).
    pub capacity:      usize,
    /// Ralloc arena identifier; `0` means unassigned.
    pub arena_id:      u8,
    /// Log-sequence number captured at allocation time (for write-ahead logging).
    pub log_seq:       u64,
    /// Whether the record represents a live (not yet freed) allocation.
    pub live:          bool,
}

// ── AllocTableStats ───────────────────────────────────────────────────────────

/// Aggregate statistics over the allocation table.
#[derive(Debug, Clone, Default)]
pub struct AllocTableStats {
    /// Number of records currently marked live.
    pub live_count:      usize,
    /// Cumulative number of insertions since the table was created.
    pub total_allocated: u64,
    /// Cumulative number of successful removals since the table was created.
    pub total_freed:     u64,
    /// Sum of `byte_len` across all currently-live records.
    pub live_bytes:      usize,
}

// ── AllocTableError ───────────────────────────────────────────────────────────

/// Errors returned by [`AllocTable`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocTableError {
    /// No record exists for the given `vm_address`.
    NotFound,
    /// A record exists but its generation does not match the caller's.
    GenerationMismatch {
        /// The generation supplied by the caller.
        expected: u64,
        /// The generation stored in the table.
        actual:   u64,
    },
    /// A live record already exists for the given `vm_address`.
    AlreadyExists,
    /// The record found is already marked dead.
    AlreadyDead,
}

impl std::fmt::Display for AllocTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocTableError::NotFound => write!(f, "alloc table: no record for vm_address"),
            AllocTableError::GenerationMismatch { expected, actual } => {
                write!(f, "alloc table: generation mismatch (expected {expected}, actual {actual})")
            }
            AllocTableError::AlreadyExists => {
                write!(f, "alloc table: a live record already exists for vm_address")
            }
            AllocTableError::AlreadyDead => {
                write!(f, "alloc table: record is already marked dead")
            }
        }
    }
}

impl std::error::Error for AllocTableError {}

// ── Inner state ───────────────────────────────────────────────────────────────

struct Inner {
    records:         HashMap<usize, AllocRecord>,
    total_allocated: u64,
    total_freed:     u64,
}

impl Inner {
    fn new() -> Self {
        Self {
            records:         HashMap::new(),
            total_allocated: 0,
            total_freed:     0,
        }
    }

    fn live_bytes(&self) -> usize {
        self.records.values().filter(|r| r.live).map(|r| r.byte_len).sum()
    }

    fn live_count(&self) -> usize {
        self.records.values().filter(|r| r.live).count()
    }
}

// ── AllocTable ────────────────────────────────────────────────────────────────

/// Thread-safe allocation table mapping VM heap slots to native handles.
///
/// All methods take `&self` (shared reference) because internal mutation is
/// managed through the inner [`Mutex`]. The coordinator may later promote the
/// lock to a `RwLock` for read-heavy workloads.
pub struct AllocTable {
    inner: Mutex<Inner>,
}

impl AllocTable {
    /// Creates a new, empty allocation table.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::new()),
        }
    }

    /// Insert a new allocation record.
    ///
    /// Fails with [`AllocTableError::AlreadyExists`] if a **live** record
    /// already exists for `record.vm_address`. Dead records at the same address
    /// are replaced (they represent a previous occupant of the slot).
    ///
    /// Updates the `total_allocated` counter on success.
    pub fn insert(&self, record: AllocRecord) -> Result<(), AllocTableError> {
        let mut guard = self.inner.lock().unwrap();
        // Reject if a live record already occupies this slot.
        if let Some(existing) = guard.records.get(&record.vm_address) {
            if existing.live {
                return Err(AllocTableError::AlreadyExists);
            }
        }
        guard.total_allocated = guard.total_allocated.saturating_add(1);
        guard.records.insert(record.vm_address, record);
        Ok(())
    }

    /// Remove an allocation record with generation validation.
    ///
    /// Returns the removed record on success. Fails if:
    /// - No record exists ([`AllocTableError::NotFound`])
    /// - The generation does not match ([`AllocTableError::GenerationMismatch`])
    ///
    /// Updates the `total_freed` counter on success.
    pub fn remove(&self, vm_address: usize, vm_generation: u64) -> Result<AllocRecord, AllocTableError> {
        let mut guard = self.inner.lock().unwrap();
        let record = guard.records.get(&vm_address).ok_or(AllocTableError::NotFound)?;
        if record.vm_generation != vm_generation {
            return Err(AllocTableError::GenerationMismatch {
                expected: vm_generation,
                actual:   record.vm_generation,
            });
        }
        let removed = guard.records.remove(&vm_address).unwrap();
        guard.total_freed = guard.total_freed.saturating_add(1);
        Ok(removed)
    }

    /// Generation-validated lookup.
    ///
    /// Returns a clone of the record if found and the generation matches.
    /// Returns `None` if the address is unknown or the generation is stale —
    /// mirroring the `Option` semantics of `TinyHeap::get_address`.
    pub fn get(&self, vm_address: usize, vm_generation: u64) -> Option<AllocRecord> {
        let guard = self.inner.lock().unwrap();
        let record = guard.records.get(&vm_address)?;
        if record.vm_generation != vm_generation {
            return None;
        }
        Some(record.clone())
    }

    /// Mark a record as dead without removing it.
    ///
    /// Useful for deferred cleanup passes. Dead records are excluded from
    /// [`all_live`][Self::all_live] but remain in the table until explicitly
    /// removed or drained.
    ///
    /// Fails if:
    /// - No record exists ([`AllocTableError::NotFound`])
    /// - The generation does not match ([`AllocTableError::GenerationMismatch`])
    /// - The record is already dead ([`AllocTableError::AlreadyDead`])
    pub fn mark_dead(&self, vm_address: usize, vm_generation: u64) -> Result<(), AllocTableError> {
        let mut guard = self.inner.lock().unwrap();
        let record = guard.records.get_mut(&vm_address).ok_or(AllocTableError::NotFound)?;
        if record.vm_generation != vm_generation {
            return Err(AllocTableError::GenerationMismatch {
                expected: vm_generation,
                actual:   record.vm_generation,
            });
        }
        if !record.live {
            return Err(AllocTableError::AlreadyDead);
        }
        record.live = false;
        Ok(())
    }

    /// Returns clones of all currently-live records.
    ///
    /// Order is unspecified.
    pub fn all_live(&self) -> Vec<AllocRecord> {
        let guard = self.inner.lock().unwrap();
        guard.records.values().filter(|r| r.live).cloned().collect()
    }

    /// Returns current aggregate statistics.
    pub fn stats(&self) -> AllocTableStats {
        let guard = self.inner.lock().unwrap();
        AllocTableStats {
            live_count:      guard.live_count(),
            total_allocated: guard.total_allocated,
            total_freed:     guard.total_freed,
            live_bytes:      guard.live_bytes(),
        }
    }

    /// Drain all records (live and dead) for shutdown.
    ///
    /// After this call the table is empty. The returned `Vec` contains every
    /// record that was present, in unspecified order. The `total_allocated` and
    /// `total_freed` counters are **not** reset; they represent cumulative
    /// lifetime totals.
    pub fn drain_for_shutdown(&self) -> Vec<AllocRecord> {
        let mut guard = self.inner.lock().unwrap();
        let drained: Vec<AllocRecord> = guard.records.drain().map(|(_, v)| v).collect();
        drained
    }

    /// Total number of records in the table, live and dead.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().records.len()
    }

    /// Returns `true` if the table contains no records.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().records.is_empty()
    }
}

impl Default for AllocTable {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(vm_address: usize, vm_generation: u64, byte_len: usize) -> AllocRecord {
        AllocRecord {
            vm_address,
            vm_generation,
            native_handle: None,
            kind: AllocKind::String,
            byte_len,
            capacity: byte_len,
            arena_id: 0,
            log_seq: 0,
            live: true,
        }
    }

    // 1. insert then get with correct generation returns Some ─────────────────

    #[test]
    fn insert_and_get() {
        let table = AllocTable::new();
        table.insert(make_record(1, 1, 64)).unwrap();

        let record = table.get(1, 1);
        assert!(record.is_some(), "expected Some for matching generation");
        let record = record.unwrap();
        assert_eq!(record.vm_address, 1);
        assert_eq!(record.vm_generation, 1);
        assert_eq!(record.byte_len, 64);
        assert!(record.live);
    }

    // 2. get with wrong generation returns None ───────────────────────────────

    #[test]
    fn get_wrong_generation() {
        let table = AllocTable::new();
        table.insert(make_record(2, 3, 32)).unwrap();

        // generation 4 is stale
        assert!(table.get(2, 4).is_none(), "stale generation should return None");
        // generation 2 is also wrong
        assert!(table.get(2, 2).is_none(), "older generation should return None");
    }

    // 3. remove, verify stats update, then get returns None ──────────────────

    #[test]
    fn remove_and_verify_freed() {
        let table = AllocTable::new();
        table.insert(make_record(3, 1, 128)).unwrap();

        let stats_before = table.stats();
        assert_eq!(stats_before.total_allocated, 1);
        assert_eq!(stats_before.total_freed, 0);
        assert_eq!(stats_before.live_bytes, 128);

        let removed = table.remove(3, 1).unwrap();
        assert_eq!(removed.vm_address, 3);

        let stats_after = table.stats();
        assert_eq!(stats_after.total_freed, 1);
        assert_eq!(stats_after.live_bytes, 0);

        // no longer findable
        assert!(table.get(3, 1).is_none());
    }

    // 4. double insert while first is live returns AlreadyExists ─────────────

    #[test]
    fn double_insert_fails() {
        let table = AllocTable::new();
        table.insert(make_record(4, 1, 16)).unwrap();

        let err = table.insert(make_record(4, 1, 16)).unwrap_err();
        assert_eq!(err, AllocTableError::AlreadyExists, "second insert of live slot should fail with AlreadyExists");
    }

    // 5. mark_dead excludes record from all_live ───────────────────────────────

    #[test]
    fn mark_dead_not_in_all_live() {
        let table = AllocTable::new();
        table.insert(make_record(5, 1, 8)).unwrap();
        table.insert(make_record(6, 1, 8)).unwrap();

        assert_eq!(table.all_live().len(), 2);

        table.mark_dead(5, 1).unwrap();

        let live = table.all_live();
        assert_eq!(live.len(), 1, "dead record should not appear in all_live");
        assert_eq!(live[0].vm_address, 6);

        // still present in the table (len includes dead)
        assert_eq!(table.len(), 2);
    }

    // 6. drain_for_shutdown empties the table ─────────────────────────────────

    #[test]
    fn drain_for_shutdown_empties() {
        let table = AllocTable::new();
        table.insert(make_record(7, 1, 4)).unwrap();
        table.insert(make_record(8, 2, 4)).unwrap();
        table.mark_dead(8, 2).unwrap(); // one live, one dead

        let drained = table.drain_for_shutdown();
        assert_eq!(drained.len(), 2, "drain should return all records");
        assert!(table.is_empty(), "table must be empty after drain");

        // cumulative counters survive the drain
        let stats = table.stats();
        assert_eq!(stats.total_allocated, 2);
    }

    // ── Additional edge-case tests ────────────────────────────────────────────

    // mark_dead on an already-dead record returns AlreadyDead
    #[test]
    fn mark_dead_twice_returns_already_dead() {
        let table = AllocTable::new();
        table.insert(make_record(9, 1, 8)).unwrap();
        table.mark_dead(9, 1).unwrap();

        let err = table.mark_dead(9, 1).unwrap_err();
        assert_eq!(err, AllocTableError::AlreadyDead);
    }

    // remove with wrong generation returns GenerationMismatch
    #[test]
    fn remove_wrong_generation_returns_mismatch() {
        let table = AllocTable::new();
        table.insert(make_record(10, 5, 8)).unwrap();

        let err = table.remove(10, 4).unwrap_err();
        assert!(
            matches!(
                err,
                AllocTableError::GenerationMismatch {
                    expected: 4,
                    actual:   5,
                }
            ),
            "unexpected error: {err}"
        );
    }

    // inserting at a previously-dead address succeeds (slot reuse)
    #[test]
    fn reinsert_after_dead_succeeds() {
        let table = AllocTable::new();
        table.insert(make_record(11, 1, 8)).unwrap();
        table.mark_dead(11, 1).unwrap();

        // new generation occupies the same slot address
        table.insert(make_record(11, 2, 16)).unwrap();
        let record = table.get(11, 2).unwrap();
        assert_eq!(record.byte_len, 16);
    }

    // stats.live_bytes tracks correctly across multiple inserts/removes
    #[test]
    fn stats_live_bytes_tracking() {
        let table = AllocTable::new();
        table.insert(make_record(20, 1, 100)).unwrap();
        table.insert(make_record(21, 1, 200)).unwrap();

        assert_eq!(table.stats().live_bytes, 300);

        table.remove(20, 1).unwrap();
        assert_eq!(table.stats().live_bytes, 200);

        table.mark_dead(21, 1).unwrap();
        assert_eq!(table.stats().live_bytes, 0);
    }
}
