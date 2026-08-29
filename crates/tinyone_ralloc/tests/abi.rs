use core::ffi::c_void;

use ralloc::{ralloc_aligned_alloc, ralloc_calloc, ralloc_free, ralloc_malloc, ralloc_realloc};

#[test]
fn malloc_family_symbols_have_c_allocator_shapes() {
    let malloc_fn: unsafe extern "C" fn(usize) -> *mut c_void = ralloc_malloc;
    let aligned_alloc_fn: unsafe extern "C" fn(usize, usize) -> *mut c_void = ralloc_aligned_alloc;
    let free_fn: unsafe extern "C" fn(*mut c_void) = ralloc_free;
    let calloc_fn: unsafe extern "C" fn(usize, usize) -> *mut c_void = ralloc_calloc;
    let realloc_fn: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void = ralloc_realloc;

    let _ = (malloc_fn, aligned_alloc_fn, free_fn, calloc_fn, realloc_fn);
}
