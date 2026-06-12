//! C ABI entrypoints for Ralloc.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::{cmp, ptr};

use crate::arena::Arena;
use crate::backend::{FixedRegionBackend, MemoryBackend, RegionRequest};
use crate::region::{self, ArenaSlot};
use crate::sync::SpinLock;

const ARENA_COUNT: usize = 4;
const ARENA_BYTES: usize = 64 * 1024;
const ARENA_ALIGN: usize = 64;
pub(crate) const MAX_NATIVE_ALIGNMENT: usize = 4096;

#[repr(align(64))]
struct StaticArenaStorage([UnsafeCell<[u8; ARENA_BYTES]>; ARENA_COUNT]);

// SAFETY: Access to each storage slot is mediated by the matching entry in
// `ARENAS`; initialization constructs one arena over each span before that
// arena can mutate it.
unsafe impl Sync for StaticArenaStorage {}

static STORAGE: StaticArenaStorage = StaticArenaStorage([const { UnsafeCell::new([0; ARENA_BYTES]) }; ARENA_COUNT]);
static ARENAS: [SpinLock<ArenaState>; ARENA_COUNT] = [const { SpinLock::new(ArenaState::new()) }; ARENA_COUNT];

struct ArenaState {
    arena: Option<Arena>,
}

impl ArenaState {
    const fn new() -> Self {
        Self { arena: None }
    }

    fn ensure_arena(&mut self, slot: ArenaSlot) -> Option<&mut Arena> {
        if self.arena.is_none() {
            let storage = STORAGE.0[slot.index()].get().cast::<u8>();
            let mut backend = FixedRegionBackend::new(storage, ARENA_BYTES);
            let region = backend.allocate_region(RegionRequest::new(ARENA_BYTES, ARENA_ALIGN)?)?;
            self.arena = Arena::from_region(region, slot, 0).ok();
        }

        self.arena.as_mut()
    }
}

fn allocate(size: usize) -> *mut c_void {
    allocate_aligned(size, crate::block::ALIGNMENT)
}

pub(crate) fn allocate_aligned(size: usize, align: usize) -> *mut c_void {
    if size == 0 {
        return ptr::null_mut();
    }
    if !is_supported_native_alignment(align) {
        return ptr::null_mut();
    }

    for (index, arena) in ARENAS.iter().enumerate() {
        let Some(mut state) = arena.try_lock() else {
            continue;
        };
        let slot = ArenaSlot::new(index);
        let Some(arena) = state.ensure_arena(slot) else {
            continue;
        };
        if let Ok(payload) = arena.allocate_aligned(size, align) {
            return payload.cast::<c_void>();
        }
    }

    for (index, arena) in ARENAS.iter().enumerate() {
        let mut state = arena.lock();
        let slot = ArenaSlot::new(index);
        let Some(arena) = state.ensure_arena(slot) else {
            continue;
        };
        if let Ok(payload) = arena.allocate_aligned(size, align) {
            return payload.cast::<c_void>();
        }
    }

    ptr::null_mut()
}

fn deallocate(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }

    let Some(record) = region::lookup(ptr.addr()) else {
        return;
    };
    let index = record.owner().index();
    if index >= ARENA_COUNT {
        return;
    }

    let mut state = ARENAS[index].lock();
    if let Some(arena) = state.arena.as_mut() {
        let _ = arena.deallocate(ptr.cast::<u8>());
    }
}

fn reallocate(ptr: *mut c_void, size: usize) -> *mut c_void {
    reallocate_aligned(ptr, size, crate::block::ALIGNMENT)
}

pub(crate) fn reallocate_aligned(ptr: *mut c_void, size: usize, align: usize) -> *mut c_void {
    if ptr.is_null() {
        return allocate_aligned(size, align);
    }
    if size == 0 {
        deallocate(ptr);
        return ptr::null_mut();
    }
    if !is_supported_native_alignment(align) {
        return ptr::null_mut();
    }

    let Some(record) = region::lookup(ptr.addr()) else {
        return ptr::null_mut();
    };
    let index = record.owner().index();
    if index >= ARENA_COUNT {
        return ptr::null_mut();
    }

    let old_size = {
        let mut state = ARENAS[index].lock();
        let Some(arena) = state.arena.as_mut() else {
            return ptr::null_mut();
        };
        let Ok(old_size) = arena.allocation_size(ptr.cast::<u8>()) else {
            return ptr::null_mut();
        };
        if old_size >= size && ptr.addr() & (align - 1) == 0 {
            return ptr;
        }
        old_size
    };

    let new_ptr = allocate_aligned(size, align).cast::<u8>();
    if new_ptr.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: `new_ptr` is a fresh allocation of at least `size` bytes, `ptr`
    // was validated as a live allocation of `old_size` bytes, and allocator
    // ownership rules make overlapping live allocations invalid.
    unsafe { ptr::copy_nonoverlapping(ptr.cast::<u8>(), new_ptr, cmp::min(old_size, size)) };

    deallocate(ptr);
    new_ptr.cast::<c_void>()
}

pub(crate) const fn is_supported_native_alignment(align: usize) -> bool {
    align != 0 && align.is_power_of_two() && align <= MAX_NATIVE_ALIGNMENT
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReallocCopyGuardSnapshot {
    owner:                 Option<ArenaSlot>,
    live_under_owner_lock: bool,
    allocation_size:       Option<usize>,
}

#[cfg(test)]
fn realloc_copy_guard_snapshot_for_tests(ptr: *mut c_void) -> ReallocCopyGuardSnapshot {
    let Some(record) = region::snapshot_lookup_for_tests(ptr.addr()) else {
        return ReallocCopyGuardSnapshot {
            owner:                 None,
            live_under_owner_lock: false,
            allocation_size:       None,
        };
    };

    let owner = record.owner();
    let allocation_size = if owner.index() < ARENA_COUNT {
        let mut state = ARENAS[owner.index()].lock();
        state
            .arena
            .as_mut()
            .and_then(|arena| arena.allocation_size(ptr.cast::<u8>()).ok())
    } else {
        None
    };

    ReallocCopyGuardSnapshot {
        owner: Some(owner),
        live_under_owner_lock: allocation_size.is_some(),
        allocation_size,
    }
}

/// Allocates `size` bytes and returns a pointer suitable for C callers.
///
/// Zero-size requests return null. Non-zero requests are routed through the
/// arena manager and return null when no arena can satisfy the allocation.
///
/// # Safety
///
/// Any returned non-null pointer must be used according to the C allocator
/// contract: it may be passed back to Ralloc's `free` or `realloc` functions
/// exactly as documented by those functions.
#[cfg_attr(feature = "cdylib", unsafe(no_mangle))]
pub unsafe extern "C" fn ralloc_malloc(size: usize) -> *mut c_void {
    allocate(size)
}

/// Allocates `size` bytes with at least `alignment` byte alignment.
///
/// Zero-size requests, invalid alignments, unsupported huge alignments, and
/// out-of-memory conditions return null. Successful allocations are compatible
/// with `ralloc_free`.
///
/// # Safety
///
/// Any returned non-null pointer must be used according to the C allocator
/// contract: it may be passed back to Ralloc's `free` function exactly once.
#[cfg_attr(feature = "cdylib", unsafe(no_mangle))]
pub unsafe extern "C" fn ralloc_aligned_alloc(alignment: usize, size: usize) -> *mut c_void {
    allocate_aligned(size, alignment)
}

/// Releases a pointer previously returned by Ralloc.
///
/// Null pointers are ignored. Non-null pointers are routed through the region
/// registry to the arena that owns the allocation.
///
/// # Safety
///
/// `ptr` must either be null or a live pointer returned by `ralloc_malloc`,
/// `ralloc_calloc`, or `ralloc_realloc` that has not already been freed.
#[cfg_attr(feature = "cdylib", unsafe(no_mangle))]
pub unsafe extern "C" fn ralloc_free(ptr: *mut c_void) {
    deallocate(ptr);
}

/// Allocates zero-initialized storage for `nmemb` elements of `size` bytes.
///
/// Multiplication overflow returns null. Successful allocations are filled
/// with zero bytes before being returned to the caller.
///
/// # Safety
///
/// Any returned non-null pointer must be used according to the same ownership
/// contract as `ralloc_malloc`.
#[cfg_attr(feature = "cdylib", unsafe(no_mangle))]
pub unsafe extern "C" fn ralloc_calloc(nmemb: usize, size: usize) -> *mut c_void {
    let Some(total) = nmemb.checked_mul(size) else {
        return ptr::null_mut();
    };

    let ptr = allocate(total);
    if !ptr.is_null() {
        // SAFETY: `ptr` is a live allocation of at least `total` bytes returned
        // by this allocator.
        unsafe { ptr::write_bytes(ptr, 0, total) };
    }

    ptr
}

/// Resizes an allocation previously returned by Ralloc.
///
/// Null pointers behave like `ralloc_malloc`; zero-size requests free the
/// allocation and return null.
///
/// # Safety
///
/// `ptr` must either be null or a live pointer returned by Ralloc that has not
/// already been freed. If this function returns null, the original allocation
/// remains owned by the caller unless `size` is zero.
#[cfg_attr(feature = "cdylib", unsafe(no_mangle))]
pub unsafe extern "C" fn ralloc_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    reallocate(ptr, size)
}

#[cfg(test)]
fn reset_for_tests() {
    for arena in &ARENAS {
        *arena.lock() = ArenaState::new();
    }
    region::clear_for_tests();
}

#[cfg(test)]
mod tests {
    use core::ptr;
    extern crate std;

    use std::thread;

    use super::{ReallocCopyGuardSnapshot, ralloc_calloc, ralloc_free, ralloc_malloc, ralloc_realloc};
    use crate::region::{self, ArenaSlot};

    #[test]
    fn abi_malloc_free_reuses_storage() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let first = unsafe { ralloc_malloc(24) };
        assert!(!first.is_null());
        unsafe { ralloc_free(first) };

        let second = unsafe { ralloc_malloc(24) };
        assert_eq!(first, second);
    }

    #[test]
    fn abi_calloc_zeroes_storage_and_rejects_overflow() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let ptr = unsafe { ralloc_calloc(8, 4) }.cast::<u8>();
        assert!(!ptr.is_null());

        for index in 0..32 {
            assert_eq!(unsafe { ptr.add(index).read() }, 0);
        }

        assert!(unsafe { ralloc_calloc(usize::MAX, 2) }.is_null());
    }

    #[test]
    fn abi_realloc_preserves_existing_bytes() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let ptr = unsafe { ralloc_malloc(8) }.cast::<u8>();
        assert!(!ptr.is_null());
        for index in 0..8 {
            unsafe { ptr.add(index).write(index as u8 + 1) };
        }

        let grown = unsafe { ralloc_realloc(ptr.cast(), 64) }.cast::<u8>();
        assert!(!grown.is_null());
        for index in 0..8 {
            assert_eq!(unsafe { grown.add(index).read() }, index as u8 + 1);
        }

        let shrunk = unsafe { ralloc_realloc(grown.cast(), 4) }.cast::<u8>();
        assert!(!shrunk.is_null());
        for index in 0..4 {
            assert_eq!(unsafe { shrunk.add(index).read() }, index as u8 + 1);
        }

        assert!(unsafe { ralloc_realloc(shrunk.cast(), 0) }.is_null());
        assert!(!unsafe { ralloc_realloc(ptr::null_mut(), 16) }.is_null());
    }

    #[test]
    fn abi_malloc_rejects_huge_request() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        assert!(unsafe { ralloc_malloc(usize::MAX) }.is_null());
    }

    #[test]
    fn abi_reports_out_of_memory_after_all_arenas_are_full() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let mut allocations = std::vec::Vec::new();
        loop {
            let ptr = unsafe { ralloc_malloc(8 * 1024) };
            if ptr.is_null() {
                break;
            }
            allocations.push(ptr);
        }

        assert!(!allocations.is_empty());
        assert!(unsafe { ralloc_malloc(8 * 1024) }.is_null());

        for ptr in allocations {
            unsafe { ralloc_free(ptr) };
        }
    }

    #[test]
    fn abi_rejects_in_region_non_payload_pointers() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let ptr = unsafe { ralloc_malloc(64) }.cast::<u8>();
        assert!(!ptr.is_null());

        let non_payload = unsafe { ptr.add(1) }.cast();
        unsafe { ralloc_free(non_payload) };
        assert!(unsafe { ralloc_realloc(non_payload, 128) }.is_null());

        let grown = unsafe { ralloc_realloc(ptr.cast(), 128) };
        assert!(!grown.is_null());
        unsafe { ralloc_free(grown) };
    }

    #[test]
    fn abi_allocates_from_additional_arena_when_primary_is_full() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let first = unsafe { ralloc_malloc(40 * 1024) };
        let second = unsafe { ralloc_malloc(40 * 1024) };

        assert!(!first.is_null());
        assert!(!second.is_null());
        assert_eq!(region::lookup(first.addr()).map(|record| record.owner()), Some(ArenaSlot::new(0)));
        assert_eq!(region::lookup(second.addr()).map(|record| record.owner()), Some(ArenaSlot::new(1)));
    }

    #[test]
    fn abi_test_hook_distinguishes_region_owner_from_live_realloc_source() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let ptr = unsafe { ralloc_malloc(32) };
        assert!(!ptr.is_null());

        assert_eq!(
            super::realloc_copy_guard_snapshot_for_tests(ptr),
            ReallocCopyGuardSnapshot {
                owner:                 Some(ArenaSlot::new(0)),
                live_under_owner_lock: true,
                allocation_size:       Some(32),
            }
        );

        unsafe { ralloc_free(ptr) };

        assert_eq!(region::snapshot_lookup_for_tests(ptr.addr()).map(|record| record.owner()), Some(ArenaSlot::new(0)));
        assert_eq!(
            super::realloc_copy_guard_snapshot_for_tests(ptr),
            ReallocCopyGuardSnapshot {
                owner:                 Some(ArenaSlot::new(0)),
                live_under_owner_lock: false,
                allocation_size:       None,
            }
        );
    }

    #[test]
    fn abi_rejects_huge_native_alignment_requests() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let page_aligned = super::allocate_aligned(1, 4096);
        assert!(!page_aligned.is_null());
        assert_eq!(page_aligned.addr() % 4096, 0);

        assert!(super::allocate_aligned(1, 8192).is_null());
    }

    #[test]
    fn abi_allows_concurrent_alloc_free_smoke() {
        let _guard = region::TEST_LOCK.lock();
        super::reset_for_tests();

        let mut workers = std::vec::Vec::new();
        for _ in 0..4 {
            workers.push(thread::spawn(|| {
                for _ in 0..64 {
                    let ptr = unsafe { ralloc_malloc(128) };
                    assert!(!ptr.is_null());
                    unsafe { ralloc_free(ptr) };
                }
            }));
        }

        for worker in workers {
            worker.join().expect("worker should not panic");
        }
    }
}
