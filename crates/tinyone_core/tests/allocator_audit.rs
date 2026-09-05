//! Adversarial regression tests for `TinyOne's` memory-related modules.
//!
//! Covers: `alloc_table`, `memory_log`, `vm_hooks`, `tiny_allocator`.
//!
//! Run with:
//!   cargo test --test `allocator_audit`
//!
//! These tests are designed to catch use-after-free risks, double-free risks,
//! generation check bypasses, race conditions, panic paths, ring-buffer safety,
//! hook dispatch safety, shutdown correctness, and integer overflow.

use std::sync::{Arc, Mutex};
use std::thread;

use tinyone::alloc_table::{AllocKind, AllocRecord, AllocTable, AllocTableError};
use tinyone::memory_log::{MemoryLog, MemoryLogEntry, OperationType};
use tinyone::tiny_allocator::{TinyAllocator, TinyAllocatorConfig, TinyAllocatorError};
use tinyone::vm_hooks::{HookRegistry, MemoryErrorPusher, MemoryEvent, VmMemoryHook};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

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

fn make_entry(seq: u64) -> MemoryLogEntry {
    MemoryLogEntry::success(seq, 0, OperationType::Allocate, 0x1000, 1, 64)
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: alloc_table_stale_generation_rejected
//
// Insert a record with gen=1, remove it, insert a new record at the same
// address (gen=2), verify that a lookup with gen=1 returns None.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn alloc_table_stale_generation_rejected() {
    let table = AllocTable::new();

    // First occupant: gen=1.
    table.insert(make_record(100, 1, 32)).unwrap();
    // gen=1 lookup succeeds.
    assert!(table.get(100, 1).is_some(), "gen=1 must be found while live");

    // Remove the first occupant.
    table.remove(100, 1).unwrap();
    assert!(table.get(100, 1).is_none(), "gen=1 must be gone after remove");

    // Second occupant: same address, incremented generation.
    table.insert(make_record(100, 2, 64)).unwrap();

    // Stale gen=1 lookup MUST return None — must not alias the new occupant.
    let stale = table.get(100, 1);
    assert!(stale.is_none(), "stale generation lookup must return None; got: {stale:?}");

    // Correct gen=2 lookup MUST succeed.
    let current = table.get(100, 2);
    assert!(current.is_some(), "gen=2 must be found");
    assert_eq!(current.unwrap().byte_len, 64, "gen=2 must return the new record");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: alloc_table_double_free_detected
//
// Insert, remove, try remove again with the same address+gen — verify the
// second remove returns an error (not a panic, not a spurious success).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn alloc_table_double_free_detected() {
    let table = AllocTable::new();
    table.insert(make_record(200, 1, 16)).unwrap();

    // First free — succeeds.
    table.remove(200, 1).unwrap();

    // Second free — must return an error, never panic.
    let result = table.remove(200, 1);
    assert!(result.is_err(), "second remove of the same address must return Err");
    let err = result.unwrap_err();
    assert!(
        matches!(err, AllocTableError::NotFound | AllocTableError::GenerationMismatch { .. }),
        "expected NotFound or GenerationMismatch on double-free; got {err:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: alloc_table_concurrent_insert_and_get
//
// Spawn 8 threads each inserting and getting their own distinct vm_address.
// Verify no data races (run with `--test-threads=8`).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn alloc_table_concurrent_insert_and_get() {
    let table = Arc::new(AllocTable::new());
    let mut handles = Vec::new();

    for t in 0u64..8 {
        let table = Arc::clone(&table);
        handles.push(thread::spawn(move || {
            // Each thread gets a distinct base address range so they don't collide.
            let base_address = (usize::try_from(t).expect("Failed to case base_address for t as a usize!")) * 1000;
            for i in 0usize..100 {
                let addr = base_address + i;
                let generation = (t * 100 + i as u64) + 1; // generation >= 1
                // Insert
                table
                    .insert(make_record(addr, generation, 8))
                    .expect("concurrent insert must not fail for distinct addresses");
                // Get — must return Some with the correct generation.
                let record = table.get(addr, generation);
                assert!(
                    record.is_some(),
                    "thread {t}: get(addr={addr}, generation={generation}) returned None immediately after insert"
                );
                assert_eq!(
                    record.unwrap().vm_generation,
                    generation,
                    "thread {t}: returned record has wrong generation"
                );
                // Remove cleanly.
                table.remove(addr, generation).expect("concurrent remove must succeed");
            }
        }));
    }

    for h in handles {
        h.join().expect("thread panicked unexpectedly");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: memory_log_ring_overflow_no_panic
//
// Create a log with capacity 4, log 100 entries; verify no panic and
// len() <= 4.  Also verify the snapshot is ordered and coherent.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn memory_log_ring_overflow_no_panic() {
    let log = MemoryLog::new(4);

    for i in 0u64..100 {
        // Must not panic regardless of how many entries we push.
        log.log(make_entry(i));
    }

    let len = log.len();
    assert!(len <= 4, "log.len() must never exceed capacity=4; got len={len}");
    assert_eq!(len, 4, "ring buffer must stay exactly at capacity after overflow");

    let snap = log.snapshot();
    assert_eq!(snap.len(), 4, "snapshot must contain exactly 4 entries");

    // The snapshot must be non-empty and the seqs must be strictly increasing
    // (oldest-first ordering).
    for i in 1..snap.len() {
        assert!(
            snap[i].seq > snap[i - 1].seq,
            "snapshot entries must be in ascending seq order; found seq[{}]={} after seq[{}]={}",
            i,
            snap[i].seq,
            i - 1,
            snap[i - 1].seq
        );
    }

    // The last entry must have seq=99 (the final push).
    assert_eq!(snap[3].seq, 99, "most recent entry must be seq=99");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 5: memory_log_concurrent_log_no_deadlock
//
// Spawn 16 threads each logging 1000 entries.  Verify no deadlock and that
// the final len() is exactly the ring capacity (all writes completed).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn memory_log_concurrent_log_no_deadlock() {
    // Use a ring big enough that we don't thrash but small enough that we
    // definitely overflow, exercising the eviction path under concurrent load.
    let log = Arc::new(MemoryLog::new(512));
    let mut handles = Vec::new();

    for t in 0u64..16 {
        let log = Arc::clone(&log);
        handles.push(thread::spawn(move || {
            for i in 0u64..1_000 {
                let ui = usize::try_from(i).expect("Failed to cast i as a usize!");
                log.log(MemoryLogEntry::success(t * 1_000 + i, t, OperationType::Allocate, 0x2000 + ui, 1, 8));
            }
        }));
    }

    // Join without a sleep-based timeout; if there is a deadlock the test
    // harness will time out via the cargo test timeout mechanism.
    for h in handles {
        h.join().expect("logging thread panicked unexpectedly");
    }

    // All 16*1000=16000 writes completed; ring saturates at 512.
    let len = log.len();
    assert_eq!(len, 512, "log must be at capacity (512) after 16 000 concurrent writes");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 6: hook_panicking_hook_does_not_propagate
//
// Register a hook that panics on on_memory_event, dispatch an event, verify
// no panic reaches the test.  Also register a counting hook after the panicking
// one to confirm dispatch continues past the panic.
// ─────────────────────────────────────────────────────────────────────────────

struct PanickingHook;
impl VmMemoryHook for PanickingHook {
    fn on_memory_event(&self, _event: &MemoryEvent) {
        panic!("intentional adversarial hook panic — must be caught by the registry");
    }
}

struct CountingHook {
    count: Mutex<u32>,
}

impl CountingHook {
    fn new() -> Arc<Self> {
        Arc::new(Self { count: Mutex::new(0) })
    }

    fn count(&self) -> u32 {
        *self.count.lock().unwrap()
    }
}

impl VmMemoryHook for CountingHook {
    fn on_memory_event(&self, _event: &MemoryEvent) {
        *self.count.lock().unwrap() += 1;
    }
}

#[test]
fn hook_panicking_hook_does_not_propagate() {
    let registry = HookRegistry::new();
    let counter = CountingHook::new();

    // Panicking hook registered FIRST — if panic propagates the test itself panics.
    registry.register(Arc::new(PanickingHook) as Arc<dyn VmMemoryHook>);
    // Counting hook registered SECOND — must still be called despite prior panic.
    registry.register(Arc::clone(&counter) as Arc<dyn VmMemoryHook>);

    // Should NOT panic; the panic is expected to be caught inside dispatch().
    registry.dispatch(MemoryEvent::Allocated {
        vm_address:    0xDEAD,
        vm_generation: 1,
        size:          64,
        type_name:     "Buffer",
    });

    // The counting hook must have received the event.
    assert_eq!(counter.count(), 1, "counting hook must be called even after prior hook panicked");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 7: tiny_allocator_shutdown_rejects_new_allocs
//
// Call shutdown_drain(), then call allocate() — verify ShutdownInProgress is
// returned (not a panic, not a silent success).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tiny_allocator_shutdown_rejects_new_allocs() {
    let alloc = TinyAllocator::with_defaults();

    // Do a normal allocation first to confirm the allocator works.
    alloc.allocate(1, 1, AllocKind::String, 64, 0).unwrap();
    alloc.free(1, 1, 0).unwrap();

    // Trigger shutdown.
    let report = alloc.shutdown_drain(0);
    assert_eq!(report.live_count, 0, "no live allocations at shutdown");

    // Post-shutdown allocations must be rejected.
    let err = alloc.allocate(2, 1, AllocKind::Array, 64, 0).unwrap_err();
    assert!(
        matches!(err, TinyAllocatorError::ShutdownInProgress),
        "expected ShutdownInProgress after shutdown_drain; got {err:?}"
    );

    // Repeated calls should also fail (idempotent rejection).
    let err2 = alloc.allocate(3, 1, AllocKind::Buffer, 8, 0).unwrap_err();
    assert!(
        matches!(err2, TinyAllocatorError::ShutdownInProgress),
        "repeated post-shutdown allocate must return ShutdownInProgress"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 8: tiny_allocator_stale_pointer_fires_hook
//
// Register MemoryErrorPusher; allocate, free with the correct gen (succeeds),
// then attempt a second free with the same (now stale) gen — verify
// has_errors() is true and the error event is StalePointer or DoubleFree.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tiny_allocator_stale_pointer_fires_hook() {
    let alloc = TinyAllocator::with_defaults();
    let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
    alloc.register_hook(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);

    // Allocate successfully.
    alloc.allocate(50, 1, AllocKind::Buffer, 128, 0).unwrap();

    // First free — correct generation.
    alloc.free(50, 1, 0).unwrap();
    assert!(!pusher.has_errors(), "no errors expected after a valid free; pusher: {:?}", pusher.drain_errors());

    // Second free — stale generation (same gen, but slot is now dead/gone).
    let err = alloc.free(50, 1, 0).unwrap_err();
    assert!(
        matches!(err, TinyAllocatorError::DoubleFree { .. } | TinyAllocatorError::GenerationMismatch { .. }),
        "second free must be rejected with DoubleFree or GenerationMismatch; got {err:?}"
    );

    // The hook MUST have received an error event.
    assert!(pusher.has_errors(), "MemoryErrorPusher must report errors after the stale/double free");

    let events = pusher.drain_errors();
    let found_error_event = events.iter().any(|e| {
        matches!(
            e,
            MemoryEvent::StalePointer { .. } | MemoryEvent::DoubleFree { .. } | MemoryEvent::AccessViolation { .. }
        )
    });
    assert!(found_error_event, "expected StalePointer/DoubleFree/AccessViolation event; got {events:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 9: tiny_allocator_log_grows_bounded
//
// Create allocator with log_capacity=10, do 100 allocations (plus matching
// frees), verify log snapshot len <= 10.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tiny_allocator_log_grows_bounded() {
    let alloc = TinyAllocator::new(TinyAllocatorConfig {
        log_capacity:          10,
        error_pusher_capacity: 256,
        enable_logging:        true,
    });

    for i in 0usize..100 {
        // Use distinct addresses so the table doesn't reject them as
        // AlreadyExists; allocate then immediately free each slot.
        alloc.allocate(i + 1, 1, AllocKind::Char, 8, 0).unwrap();
        alloc.free(i + 1, 1, 0).unwrap();
    }

    let snap = alloc.log_snapshot();
    assert!(snap.len() <= 10, "log snapshot must not exceed log_capacity=10; got len={}", snap.len());
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 10: ralloc_vm_alloc_round_trips_through_the_real_allocator
//
// `ralloc` is now a real dependency of the `tinylang` crate, so this test
// exercises `ralloc::VmAllocator` directly (not through `TinyAllocator`) to
// confirm the dependency wiring resolves correctly from this integration
// test's own compilation unit, not just from unit tests inside the Ralloc
// crate itself.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ralloc_vm_alloc_round_trips_through_the_real_allocator() {
    use ralloc::VmAllocator;

    let vm = VmAllocator::global();
    let mut alloc = vm.allocate(32).expect("allocation should succeed");
    assert_eq!(alloc.len(), 32);
    alloc.as_mut_slice().fill(0x5A);
    assert!(alloc.as_slice().iter().all(|&b| b == 0x5A));
    vm.deallocate(alloc); // consumes `alloc` by value
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 10b: ralloc_vm_alloc_double_deallocate_rejected (type-level compile check)
//
// VmAllocation is !Clone + !Copy.  VmAllocator::deallocate() consumes the
// VmAllocation by value (move semantics).  Therefore attempting to call
// deallocate twice on the same handle is a *compile-time error* — the
// borrow checker prevents it entirely.  There is no runtime test required
// or possible, because the second call would not compile.
//
// This test documents the compile-time guarantee rather than exercising
// runtime behavior.  It is marked #[ignore] because it validates a
// type-system property, not a runtime path.
//
// If you want to verify the compile error manually, try adding:
//
//   let alloc = vm.allocate(32).unwrap();
//   vm.deallocate(alloc);   // consumes alloc
//   vm.deallocate(alloc);   // ERROR: use of moved value
//
// to a file and observing the rustc error E0382.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "compile-time guarantee: VmAllocation is !Clone+!Copy; second deallocate is E0382"]
fn ralloc_vm_alloc_double_deallocate_rejected() {
    // The move-only design of VmAllocation (no Clone, no Copy, no Drop impl,
    // consumed by deallocate) makes double-free a zero-cost compile-time
    // guarantee. No runtime sentinel, no generation check, no flag — Rust's
    // ownership system enforces it structurally. See test 10 above for the
    // real, runnable single-deallocate round trip.
}

// ─────────────────────────────────────────────────────────────────────────────
// Bonus adversarial tests
// ─────────────────────────────────────────────────────────────────────────────

// ── Bonus A: shutdown does not reject free of already-allocated slot ──────────
//
// The documented behavior is: after shutdown_drain, allocate() is rejected but
// free() may still be called (to allow cleanup).  However, since shutdown_drain
// drains the table, there's nothing left to free.  Verify free() returns an
// appropriate error (not ShutdownInProgress, not a panic).

#[test]
fn tiny_allocator_post_shutdown_free_returns_not_found() {
    let alloc = TinyAllocator::with_defaults();
    alloc.allocate(99, 1, AllocKind::Map, 32, 0).unwrap();

    // Shutdown drains the table (addr 99 is now gone from the table).
    alloc.shutdown_drain(0);

    // Attempt to free an address that was drained (not explicitly freed).
    let err = alloc.free(99, 1, 0).unwrap_err();
    assert!(
        matches!(err, TinyAllocatorError::DoubleFree { .. } | TinyAllocatorError::NotFound { .. }),
        "post-shutdown free of drained address must return DoubleFree or NotFound; got {err:?}"
    );
}

// ── Bonus B: reallocate with wrong generation fires hook and returns error ────

#[test]
fn tiny_allocator_reallocate_stale_gen_returns_error() {
    let alloc = TinyAllocator::with_defaults();
    let pusher = Arc::new(MemoryErrorPusher::with_default_capacity());
    alloc.register_hook(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);

    alloc.allocate(77, 1, AllocKind::Vec, 64, 0).unwrap();

    // Reallocate with wrong generation — must error, not panic, not succeed silently.
    let err = alloc.reallocate(77, 999, 128, 0).unwrap_err();
    assert!(
        matches!(err, TinyAllocatorError::GenerationMismatch { .. } | TinyAllocatorError::NotFound { .. }),
        "reallocate with wrong gen must return GenerationMismatch or NotFound; got {err:?}"
    );

    // Clean up.
    alloc.free(77, 1, 0).unwrap();
}

// ── Bonus B2: TinyAllocator's native allocation is real, not a placeholder ──
//
// A single Ralloc allocation cannot exceed one arena
// (`ralloc::VmAllocator::max_allocation_size()`). Filling every arena with
// one allocation each, then requesting one more of the same size, proves
// `allocate()` is now backed by a real, capacity-bounded native allocator
// (Phase 2 always succeeded regardless of size). Freeing one of the four and
// retrying proves `free()` genuinely returns native space to Ralloc rather
// than only updating table bookkeeping.

#[test]
fn tiny_allocator_native_allocation_is_real_and_reclaimed_by_free() {
    let alloc = TinyAllocator::with_defaults();

    // Large enough that only one such allocation fits per arena, small
    // enough that it still fits within a single arena.
    let chunk = ralloc::VmAllocator::max_allocation_size() * 3 / 4;
    let arena_count = ralloc::VmAllocator::capacity() / ralloc::VmAllocator::max_allocation_size();

    let mut filled = Vec::new();
    for i in 0..arena_count {
        let result = alloc
            .allocate(9_000 + i, 1, AllocKind::Buffer, chunk, 0)
            .unwrap_or_else(|e| panic!("arena {i} of {arena_count} should have room: {e:?}"));
        filled.push(result);
    }

    // Every arena now holds one `chunk`-sized allocation; there is no room
    // left anywhere in the pool for another one.
    let err = alloc
        .allocate(9_999, 1, AllocKind::Buffer, chunk, 0)
        .expect_err("no arena should have room for one more chunk-sized allocation");
    assert!(
        matches!(err, TinyAllocatorError::NativeAllocFailed),
        "expected NativeAllocFailed once every arena is full, got {err:?}"
    );

    // Freeing one allocation returns its arena's space to Ralloc.
    let freed = filled.pop().unwrap();
    alloc.free(freed.vm_address, freed.vm_generation, 0).unwrap();

    let reclaimed = alloc
        .allocate(9_999, 1, AllocKind::Buffer, chunk, 0)
        .expect("allocation should succeed once free() has reclaimed an arena's space");

    // Clean up so this test doesn't leak native pool space that other tests
    // in this binary may depend on.
    alloc.free(reclaimed.vm_address, reclaimed.vm_generation, 0).unwrap();
    for result in filled {
        alloc.free(result.vm_address, result.vm_generation, 0).unwrap();
    }
}

// ── Bonus C: AllocTable.get after mark_dead returns None via all_live but ──
//             the record is still in the table and get() still succeeds
//             (get is not filtered by liveness — consistent with design).

#[test]
fn alloc_table_get_dead_record_still_returns_some() {
    let table = AllocTable::new();
    table.insert(make_record(300, 1, 32)).unwrap();
    table.mark_dead(300, 1).unwrap();

    // get() only checks address and generation, not liveness — should still
    // return the record (callers use `live` field to decide what to do).
    let record = table.get(300, 1);
    assert!(record.is_some(), "get() on a dead (but not removed) record must return Some; got None");
    assert!(!record.unwrap().live, "returned dead record must have live=false");

    // all_live() must NOT include the dead record.
    assert!(table.all_live().is_empty(), "dead record must not appear in all_live()");
}

// ── Bonus D: memory_log disabled flag suppresses concurrent writes atomically ─

#[test]
fn memory_log_disable_suppresses_concurrent_writes() {
    let log = Arc::new(MemoryLog::new(1024));
    log.disable();

    let mut handles = Vec::new();
    for t in 0u64..4 {
        let log = Arc::clone(&log);
        handles.push(thread::spawn(move || {
            for i in 0u64..500 {
                log.log(make_entry(t * 500 + i));
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(log.len(), 0, "no entries must be written while logging is disabled; got len={}", log.len());

    // Re-enable and confirm writes work again.
    log.enable();
    log.log(make_entry(9999));
    assert_eq!(log.len(), 1, "log must accept entries after re-enable");
}

// ── Bonus E: AllocTable stats live_bytes never underflows ─────────────────────

#[test]
fn alloc_table_stats_live_bytes_never_underflows() {
    let table = AllocTable::new();

    // Insert and remove several records of varying sizes.
    for i in 0u64..20 {
        let ui = usize::try_from(i).expect("Failed to cast i as a usize!");
        let size = (ui + 1) * 8;
        table.insert(make_record(ui, i + 1, size)).unwrap();
    }
    assert!(table.stats().live_bytes > 0);

    for i in 0u64..20 {
        let ui = usize::try_from(i).expect("Failed to cast i as a usize!");
        table.remove(ui, i + 1).unwrap();
        // After each remove live_bytes must still be >= 0 (it's usize so can't
        // go negative, but if the accounting is wrong it could panic in debug
        // mode via wrapping_sub — or in release mode silently wrap).
        let stats = table.stats();
        // usize can't be negative, but we can check it's not an astronomically
        // wrong value caused by wrapping subtraction.
        assert!(stats.live_bytes < usize::MAX / 2, "live_bytes appears to have wrapped; got {}", stats.live_bytes);
    }

    assert_eq!(table.stats().live_bytes, 0, "live_bytes must be 0 after all frees");
}

// ── Bonus F: HookRegistry concurrent register+dispatch does not deadlock ───────

#[test]
fn hook_registry_concurrent_register_and_dispatch_no_deadlock() {
    let registry = Arc::new(HookRegistry::new());
    let mut handles = Vec::new();

    // 4 threads register hooks concurrently.
    for _ in 0..4 {
        let reg = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            let counter = CountingHook::new();
            reg.register(Arc::clone(&counter) as Arc<dyn VmMemoryHook>);
        }));
    }

    // 4 threads dispatch events concurrently.
    for _ in 0..4 {
        let reg = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                reg.dispatch(MemoryEvent::Allocated {
                    vm_address:    0x5000,
                    vm_generation: 1,
                    size:          16,
                    type_name:     "Char",
                });
            }
        }));
    }

    for h in handles {
        h.join().expect("registry concurrent thread panicked");
    }
}

// ── Bonus G: TinyAllocator zero-size allocation is rejected pre-shutdown ───────

#[test]
fn tiny_allocator_zero_size_rejected() {
    let alloc = TinyAllocator::with_defaults();
    let err = alloc.allocate(1, 1, AllocKind::Buffer, 0, 0).unwrap_err();
    assert!(
        matches!(err, TinyAllocatorError::InvalidSize { size: 0 }),
        "zero-size allocation must be rejected with InvalidSize{{size:0}}; got {err:?}"
    );
}

// ── Bonus H: MemoryErrorPusher capacity boundary — exactly at capacity ─────────

#[test]
fn memory_error_pusher_exactly_at_capacity_no_panic() {
    let pusher = Arc::new(MemoryErrorPusher::new(3));
    let registry = HookRegistry::new();
    registry.register(Arc::clone(&pusher) as Arc<dyn VmMemoryHook>);

    // Push exactly 3 error events.
    for i in 0u64..3 {
        let ui = usize::try_from(i).expect("Failed to cast i as a usize!");
        registry.dispatch(MemoryEvent::StalePointer {
            vm_address:   ui,
            expected_gen: 1,
            actual_gen:   2,
        });
    }
    assert_eq!(pusher.error_count(), 3, "pusher must hold exactly 3 events");

    // Push one more — must evict oldest without panic.
    registry.dispatch(MemoryEvent::DoubleFree {
        vm_address:    99,
        vm_generation: 1,
    });
    assert_eq!(pusher.error_count(), 3, "pusher must still hold exactly 3 events after eviction");

    // Drain and verify oldest was evicted.
    let events = pusher.drain_errors();
    assert_eq!(events.len(), 3);
    // The first event (vm_address=0, StalePointer) should have been evicted.
    let first_addr_is_zero = matches!(&events[0], MemoryEvent::StalePointer { vm_address: 0, .. });
    assert!(
        !first_addr_is_zero,
        "vm_address=0 StalePointer should have been evicted; events[0]: {:?}",
        events[0]
    );
}

// ── Bonus I: AllocTable concurrent mixed operations (insert+remove+get) ───────

#[test]
fn alloc_table_concurrent_mixed_operations() {
    // Each thread operates on its own disjoint address range to avoid
    // intentional cross-thread collisions; we are testing lock safety, not
    // contention resolution.
    let table = Arc::new(AllocTable::new());
    let mut handles = Vec::new();

    for t in 0u64..8 {
        let table = Arc::clone(&table);
        handles.push(thread::spawn(move || {
            let tu = usize::try_from(t).expect("Failed to cast t as a usize!");
            let base = (tu) * 500;
            for i in 0..50usize {
                let addr = base + i;
                let generation = t + 1;
                table.insert(make_record(addr, generation, 32)).unwrap();
                // Interleave gets between inserts.
                assert!(table.get(addr, generation).is_some());
                table.remove(addr, generation).unwrap();
                // After remove, get must return None.
                assert!(table.get(addr, generation).is_none());
            }
        }));
    }

    for h in handles {
        h.join().expect("mixed-ops thread panicked");
    }

    // Table must be empty after all concurrent removes complete.
    assert!(table.is_empty(), "table must be empty after all concurrent removes");
}

// ── Bonus J: allocate_owned records bookkeeping without a native table entry ──
//
// `HeapData::String`/`Buffer`/`CharBuffer` now own their real `VmAllocation`
// directly (via `RallocBytes`), so `TinyHeap` records their bookkeeping
// through `TinyAllocator::allocate_owned` instead of `allocate` — this must
// NOT make a second real Ralloc allocation (that would double-book the
// arena for the most common heap object kinds), and the resulting record's
// `native_handle` must be `None` so `free()`/`shutdown_drain()`'s existing
// "nothing to release here" branch applies without any special-casing.

#[test]
fn tiny_allocator_allocate_owned_records_bookkeeping_without_native_entry() {
    let alloc = TinyAllocator::with_defaults();

    let before = alloc.stats();
    let result = alloc
        .allocate_owned(42, 1, AllocKind::String, 13, 0)
        .expect("allocate_owned should succeed");
    assert_eq!(result.vm_address, 42);
    assert_eq!(result.effective_size, 13);

    let after_alloc = alloc.stats();
    assert_eq!(after_alloc.live_count, before.live_count + 1);
    assert_eq!(after_alloc.live_bytes, before.live_bytes + 13);
    assert_eq!(after_alloc.total_allocated, before.total_allocated + 1);

    // free() must succeed even though allocate_owned never touched the
    // native side table (no VmAllocation to release here — RallocBytes
    // owns and releases the real memory itself).
    alloc.free(42, 1, 0).expect("free should succeed for an owned record");

    let after_free = alloc.stats();
    assert_eq!(after_free.live_count, before.live_count);
    assert_eq!(after_free.live_bytes, before.live_bytes);
    assert_eq!(after_free.total_freed, before.total_freed + 1);
}
