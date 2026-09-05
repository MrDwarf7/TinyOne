//! Move-only allocation handles for VM-integrated callers (e.g. `TinyOne`).
//!
//! `VmAllocator` is a thin, `Result`-returning wrapper over [`RallocBuffer`].
//! `VmAllocation` deliberately has no `Drop` impl: ownership transfer is
//! enforced structurally (the type is `!Clone + !Copy`), and release is
//! always an explicit [`VmAllocator::deallocate`] call. This mirrors a
//! manual-free heap model — forgetting to deallocate leaks, exactly like a
//! real `malloc`/`free` mismatch, rather than silently freeing on scope exit.

use core::mem::ManuallyDrop;

use crate::buffer::{RallocBuffer, RallocError};
use crate::ralloc::{ARENA_BYTES, ARENA_COUNT};

/// Owned, move-only allocation handle backed by Ralloc.
///
/// `VmAllocation` is `!Clone + !Copy` and has no `Drop` impl. The only way to
/// release it is [`VmAllocator::deallocate`], which consumes it by value.
pub struct VmAllocation(ManuallyDrop<RallocBuffer>);

impl VmAllocation {
    /// Returns the number of bytes in the allocation.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the allocation has length zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the allocation as an immutable byte slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    /// Returns the allocation as a mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.0.as_mut_slice()
    }

    /// Returns the allocation pointer for identity checks and FFI handoff.
    #[must_use]
    pub fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }
}

/// Global entry point for VM-integrated allocation.
///
/// `VmAllocator` is a stateless, zero-sized handle onto Ralloc's shared
/// static arena pool; every method delegates to the same process-wide
/// storage regardless of which `VmAllocator` value is used to call it.
pub struct VmAllocator(());

impl VmAllocator {
    /// Returns the global `VmAllocator` instance.
    #[must_use]
    pub fn global() -> &'static VmAllocator {
        static INSTANCE: VmAllocator = VmAllocator(());
        &INSTANCE
    }

    /// Allocates `len` bytes at Ralloc's default native alignment.
    ///
    /// # Errors
    ///
    /// Returns [`RallocError::OutOfMemory`] if the allocator cannot satisfy the
    /// request, or [`RallocError::InvalidAlignment`] if Ralloc's default
    /// alignment is unsupported by the native allocator. A zero-length
    /// allocation always succeeds.
    pub fn allocate(&self, len: usize) -> Result<VmAllocation, RallocError> {
        RallocBuffer::try_new(len).map(|buffer| VmAllocation(ManuallyDrop::new(buffer)))
    }

    /// Allocates `len` bytes with at least `align` byte alignment.
    ///
    /// # Errors
    ///
    /// Returns [`RallocError::InvalidAlignment`] if `align` is not a supported
    /// native alignment, or [`RallocError::OutOfMemory`] if the allocator
    /// cannot satisfy a non-zero `len` request. A zero-length allocation always
    /// succeeds.
    pub fn allocate_aligned(&self, len: usize, align: usize) -> Result<VmAllocation, RallocError> {
        RallocBuffer::try_new_aligned(len, align).map(|buffer| VmAllocation(ManuallyDrop::new(buffer)))
    }

    /// Releases a `VmAllocation`, consuming it by value.
    ///
    /// Attempting to deallocate the same handle twice does not compile
    /// (`VmAllocation` is moved into this call), so double-free is a
    /// compile-time error rather than a runtime check.
    pub fn deallocate(&self, alloc: VmAllocation) {
        let VmAllocation(buffer) = alloc;
        // Dropping the recovered `RallocBuffer` releases it via `ralloc_free`.
        let _ = ManuallyDrop::into_inner(buffer);
    }

    /// Resizes `alloc` to `new_len`, preserving the first `min(old_len,
    /// new_len)` bytes.
    ///
    /// On failure, returns the original `alloc` unchanged alongside the
    /// error — matching `realloc(3)` semantics — so the caller never loses
    /// track of a live allocation.
    ///
    /// # Errors
    ///
    /// Returns `Err((original_alloc, [``RallocError::OutOfMemory``]))` if the
    /// allocator cannot reallocate to `new_len` bytes. The original allocation
    /// is returned unchanged so the caller still owns it.
    pub fn reallocate(&self, alloc: VmAllocation, new_len: usize) -> Result<VmAllocation, (VmAllocation, RallocError)> {
        let VmAllocation(buffer) = alloc;
        let mut buffer = ManuallyDrop::into_inner(buffer);
        match buffer.try_resize(new_len) {
            Ok(()) => Ok(VmAllocation(ManuallyDrop::new(buffer))),
            Err(error) => Err((VmAllocation(ManuallyDrop::new(buffer)), error)),
        }
    }

    /// Returns the total size, in bytes, of Ralloc's static arena pool.
    ///
    /// This is the hard upper bound on the sum of all live `VmAllocation`
    /// bytes at any one time, process-wide.
    #[must_use]
    pub const fn capacity() -> usize {
        ARENA_COUNT * ARENA_BYTES
    }

    /// Returns the size, in bytes, of a single arena.
    ///
    /// A single allocation cannot span multiple arenas, so this is the hard
    /// upper bound on the size of any one `VmAllocation`, not just on the
    /// process-wide total returned by [`VmAllocator::capacity`].
    #[must_use]
    pub const fn max_allocation_size() -> usize {
        ARENA_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ralloc, region};

    #[test]
    fn allocate_and_deallocate_round_trips() {
        let _guard = region::TEST_LOCK.lock();
        ralloc::reset_for_tests();
        let vm = VmAllocator::global();
        let mut alloc = vm.allocate(64).expect("allocation should succeed");
        assert_eq!(alloc.len(), 64);
        alloc.as_mut_slice().fill(0xAB);
        assert!(alloc.as_slice().iter().all(|&b| b == 0xAB));
        vm.deallocate(alloc);
    }

    #[test]
    fn reallocate_preserves_existing_bytes() {
        let _guard = region::TEST_LOCK.lock();
        ralloc::reset_for_tests();
        let vm = VmAllocator::global();
        let mut alloc = vm.allocate(16).expect("allocation should succeed");
        alloc.as_mut_slice().copy_from_slice(&[7u8; 16]);

        let alloc = vm
            .reallocate(alloc, 32)
            .unwrap_or_else(|(_, error)| panic!("reallocate should succeed: {error:?}"));
        assert_eq!(alloc.len(), 32);
        assert_eq!(&alloc.as_slice()[..16], &[7u8; 16]);

        vm.deallocate(alloc);
    }

    #[test]
    fn reallocate_failure_returns_original_untouched() {
        let _guard = region::TEST_LOCK.lock();
        ralloc::reset_for_tests();
        let vm = VmAllocator::global();
        let alloc = vm.allocate(8).expect("allocation should succeed");

        // A request far beyond the entire arena pool must fail without
        // disturbing the original allocation's contents or ownership.
        match vm.reallocate(alloc, VmAllocator::capacity() * 4) {
            Ok(_) => panic!("reallocate should have failed"),
            Err((original, _error)) => {
                assert_eq!(original.len(), 8);
                vm.deallocate(original);
            }
        }
    }

    #[test]
    fn allocate_past_capacity_returns_err_not_panic() {
        let _guard = region::TEST_LOCK.lock();
        ralloc::reset_for_tests();
        let vm = VmAllocator::global();
        let result = vm.allocate(VmAllocator::capacity() * 2);
        assert!(result.is_err(), "oversized allocation must error, not panic");
    }

    #[test]
    fn max_allocation_size_divides_capacity_evenly() {
        assert_eq!(
            VmAllocator::capacity() % VmAllocator::max_allocation_size(),
            0,
            "capacity should be a whole number of arenas"
        );
        assert!(VmAllocator::max_allocation_size() < VmAllocator::capacity());
    }
}
