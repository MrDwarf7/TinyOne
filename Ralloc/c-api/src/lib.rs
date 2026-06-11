//! C-linkable Ralloc artifacts.

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unused_must_use)]
#![deny(missing_docs)]

use core::ffi::c_void;
#[cfg(not(test))]
use core::panic::PanicInfo;

/// Allocates `size` bytes and returns a C-compatible pointer.
///
/// # Safety
///
/// The returned pointer follows Ralloc's C allocator contract and must be
/// released with `ralloc_free` or resized with `ralloc_realloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ralloc_malloc(size: usize) -> *mut c_void {
    // SAFETY: This wrapper preserves the C ABI contract and delegates the
    // caller's allocation request to the allocator core unchanged.
    unsafe { ralloc_core::ralloc_malloc(size) }
}

/// Allocates `size` bytes with at least `alignment` byte alignment.
///
/// # Safety
///
/// The returned pointer follows Ralloc's C allocator contract and must be
/// released with `ralloc_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ralloc_aligned_alloc(alignment: usize, size: usize) -> *mut c_void {
    // SAFETY: This wrapper preserves the C ABI contract and delegates the
    // caller's alignment and size request to the allocator core unchanged.
    unsafe { ralloc_core::ralloc_aligned_alloc(alignment, size) }
}

/// Releases a pointer previously returned by Ralloc.
///
/// # Safety
///
/// `ptr` must be null or a live pointer returned by Ralloc that has not already
/// been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ralloc_free(ptr: *mut c_void) {
    // SAFETY: This wrapper preserves the C ABI contract and delegates the
    // caller-provided pointer to the allocator core unchanged.
    unsafe { ralloc_core::ralloc_free(ptr) };
}

/// Allocates zeroed storage for `nmemb * size` bytes.
///
/// # Safety
///
/// The returned pointer follows Ralloc's C allocator contract and must be
/// released with `ralloc_free` or resized with `ralloc_realloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ralloc_calloc(nmemb: usize, size: usize) -> *mut c_void {
    // SAFETY: This wrapper preserves the C ABI contract and delegates the
    // caller's allocation request to the allocator core unchanged.
    unsafe { ralloc_core::ralloc_calloc(nmemb, size) }
}

/// Resizes an allocation previously returned by Ralloc.
///
/// # Safety
///
/// `ptr` must be null or a live pointer returned by Ralloc that has not already
/// been freed. If this returns null, the original allocation remains owned by
/// the caller unless `size` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ralloc_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: This wrapper preserves the C ABI contract and delegates the
    // caller-provided pointer and size to the allocator core unchanged.
    unsafe { ralloc_core::ralloc_realloc(ptr, size) }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {}
}
