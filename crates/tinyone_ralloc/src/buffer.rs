//! Native owned allocation handles for Rust callers.

use core::cell::Cell;
use core::ffi::c_void;
use core::marker::PhantomData;
use core::mem::{self, ManuallyDrop};
use core::ops::{Deref, DerefMut};
use core::ptr::{self, NonNull};
use core::slice;

use crate::ralloc::{allocate_aligned, is_supported_native_alignment, ralloc_free, reallocate_aligned};

/// Allocation failure or raw-handle validation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RallocError {
    /// The allocator could not satisfy the request.
    OutOfMemory,
    /// The requested allocation alignment is not supported.
    InvalidAlignment,
    /// Raw ownership recovery received a null or otherwise invalid pointer.
    InvalidRawParts,
}

/// Owned byte allocation backed by Ralloc.
///
/// `RallocBuffer` gives Rust callers an ownership-based API over the allocator:
/// freeing happens on drop, resizing requires `&mut self`, and byte access is
/// exposed through Rust slices instead of requiring callers to manage raw
/// pointer lifetimes.
pub struct RallocBuffer {
    ptr:       NonNull<u8>,
    len:       usize,
    align:     usize,
    _not_sync: PhantomData<Cell<()>>,
}

// SAFETY: `RallocBuffer` owns its allocation, and moving ownership to another
// thread transfers the only safe mutable access path with it. Shared cross-thread
// access is intentionally not provided because `Cell` makes the type `!Sync`.
unsafe impl Send for RallocBuffer {}

impl RallocBuffer {
    /// Allocates a byte buffer of `len` bytes.
    ///
    /// A zero-length buffer does not allocate and always succeeds.
    pub fn new(len: usize) -> Option<Self> {
        Self::try_new(len).ok()
    }

    /// Allocates a byte buffer of `len` bytes, returning an explicit error on
    /// allocation failure.
    pub fn try_new(len: usize) -> Result<Self, RallocError> {
        Self::try_new_aligned(len, crate::block::ALIGNMENT)
    }

    /// Allocates a byte buffer of `len` bytes with at least `align` byte
    /// alignment.
    pub fn try_new_aligned(len: usize, align: usize) -> Result<Self, RallocError> {
        if !is_supported_native_alignment(align) {
            return Err(RallocError::InvalidAlignment);
        }

        if len == 0 {
            return Ok(Self::empty_with_alignment(align));
        }

        let ptr = allocate_aligned(len, align).cast::<u8>();
        let Some(ptr) = NonNull::new(ptr) else {
            return Err(RallocError::OutOfMemory);
        };

        Ok(Self {
            ptr,
            len,
            align,
            _not_sync: PhantomData,
        })
    }

    /// Returns the number of bytes in the buffer.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the buffer has length zero.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the buffer as an immutable byte slice.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` is either a live allocation of `len` bytes or a dangling
        // non-null pointer with `len == 0`.
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// Returns the buffer as a mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `&mut self` proves unique access to the owned allocation, and
        // `ptr` is either live for `len` bytes or dangling with `len == 0`.
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// Returns the allocation pointer for identity checks and FFI handoff.
    pub const fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr().cast_const()
    }

    /// Resizes the buffer, preserving the first `min(old_len, new_len)` bytes.
    ///
    /// Returns `false` if allocation fails; on failure the original buffer is
    /// left unchanged.
    pub fn resize(&mut self, new_len: usize) -> bool {
        self.try_resize(new_len).is_ok()
    }

    /// Resizes the buffer, preserving the first `min(old_len, new_len)` bytes.
    ///
    /// On error the original buffer remains unchanged.
    pub fn try_resize(&mut self, new_len: usize) -> Result<(), RallocError> {
        if new_len == self.len {
            return Ok(());
        }

        if new_len == 0 {
            self.deallocate();
            self.ptr = NonNull::dangling();
            self.len = 0;
            return Ok(());
        }

        if self.len == 0 {
            let buffer = Self::try_new_aligned(new_len, self.align)?;
            self.ptr = buffer.ptr;
            self.len = buffer.len;
            self.align = buffer.align;
            core::mem::forget(buffer);
            return Ok(());
        }

        let resized = reallocate_aligned(self.ptr.as_ptr().cast::<c_void>(), new_len, self.align).cast::<u8>();
        let Some(ptr) = NonNull::new(resized) else {
            return Err(RallocError::OutOfMemory);
        };

        self.ptr = ptr;
        self.len = new_len;
        Ok(())
    }

    /// Consumes the buffer and returns its raw allocation pointer and length.
    ///
    /// The caller becomes responsible for passing the returned parts back to
    /// `RallocBuffer::from_raw_parts` exactly once, or otherwise freeing them
    /// according to Ralloc's allocator contract.
    pub fn into_raw_parts(self) -> (*mut u8, usize) {
        let this = ManuallyDrop::new(self);
        (this.ptr.as_ptr(), this.len)
    }

    /// Reclaims ownership of raw parts produced by `into_raw_parts`.
    ///
    /// # Safety
    ///
    /// `ptr` and `len` must come from a previous `RallocBuffer::into_raw_parts`
    /// call, must not have been freed, and must not already be owned by another
    /// handle.
    pub unsafe fn from_raw_parts(ptr: *mut u8, len: usize) -> Result<Self, RallocError> {
        if len == 0 {
            return Ok(Self::empty());
        }

        let Some(ptr) = NonNull::new(ptr) else {
            return Err(RallocError::InvalidRawParts);
        };

        Ok(Self {
            ptr,
            len,
            align: crate::block::ALIGNMENT,
            _not_sync: PhantomData,
        })
    }

    fn empty() -> Self {
        Self::empty_with_alignment(crate::block::ALIGNMENT)
    }

    fn empty_with_alignment(align: usize) -> Self {
        Self {
            ptr: NonNull::dangling(),
            len: 0,
            align,
            _not_sync: PhantomData,
        }
    }

    fn deallocate(&mut self) {
        if self.len == 0 {
            return;
        }

        // SAFETY: `ptr` is a live allocation owned by this buffer.
        unsafe { ralloc_free(self.ptr.as_ptr().cast::<c_void>()) };
    }
}

impl Drop for RallocBuffer {
    fn drop(&mut self) {
        self.deallocate();
    }
}

/// Owned typed allocation backed by Ralloc.
///
/// `RallocBox<T>` owns exactly one initialized `T`. Dropping the handle drops
/// the value first and then releases the allocator storage.
pub struct RallocBox<T> {
    ptr:       NonNull<T>,
    allocated: bool,
    _not_sync: PhantomData<Cell<T>>,
}

// SAFETY: Moving `RallocBox<T>` to another thread transfers ownership of the
// contained value. This is sound when `T` itself may be sent across threads.
unsafe impl<T: Send> Send for RallocBox<T> {}

impl<T> RallocBox<T> {
    /// Allocates storage and moves `value` into it.
    pub fn new(value: T) -> Option<Self> {
        Self::try_new(value).ok()
    }

    /// Allocates storage and moves `value` into it, returning an explicit error
    /// if Ralloc cannot safely store this type.
    pub fn try_new(value: T) -> Result<Self, RallocError> {
        if mem::size_of::<T>() == 0 {
            return Ok(Self {
                ptr:       NonNull::dangling(),
                allocated: false,
                _not_sync: PhantomData,
            });
        }

        let ptr = allocate_aligned(mem::size_of::<T>(), mem::align_of::<T>()).cast::<T>();
        let Some(ptr) = NonNull::new(ptr) else {
            return Err(RallocError::OutOfMemory);
        };

        // SAFETY: `ptr` points to allocator-owned storage of at least
        // `size_of::<T>()` bytes, and this handle becomes its owner.
        unsafe { ptr.as_ptr().write(value) };

        Ok(Self {
            ptr,
            allocated: true,
            _not_sync: PhantomData,
        })
    }

    /// Returns an immutable reference to the contained value.
    pub fn get(&self) -> &T {
        // SAFETY: `ptr` points to an initialized `T` owned by this handle.
        unsafe { self.ptr.as_ref() }
    }

    /// Returns a mutable reference to the contained value.
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: `&mut self` proves unique access to the initialized `T`.
        unsafe { self.ptr.as_mut() }
    }

    /// Returns the owned value pointer.
    pub const fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr().cast_const()
    }

    /// Moves the contained value out and releases the allocation.
    pub fn into_inner(self) -> T {
        let this = ManuallyDrop::new(self);
        // SAFETY: `ptr` points to an initialized `T`, and `ManuallyDrop`
        // prevents `Drop` from dropping it a second time.
        let value = unsafe { ptr::read(this.ptr.as_ptr()) };

        if this.allocated {
            // SAFETY: The value has been moved out, but the allocation is still
            // owned by this handle and must be released exactly once.
            unsafe { ralloc_free(this.ptr.as_ptr().cast::<c_void>()) };
        }

        value
    }

    /// Consumes the box and returns the owned raw pointer.
    ///
    /// The caller becomes responsible for passing the pointer back to
    /// `RallocBox::from_raw` exactly once, or otherwise dropping the value and
    /// freeing the allocation according to Ralloc's allocator contract.
    pub fn into_raw(self) -> *mut T {
        let this = ManuallyDrop::new(self);
        this.ptr.as_ptr()
    }

    /// Reclaims ownership of a pointer produced by `into_raw`.
    ///
    /// # Safety
    ///
    /// `ptr` must come from a previous `RallocBox<T>::into_raw` call, must point
    /// to an initialized `T`, must not have been freed, and must not already be
    /// owned by another handle.
    pub unsafe fn from_raw(ptr: *mut T) -> Result<Self, RallocError> {
        let Some(ptr) = NonNull::new(ptr) else {
            return Err(RallocError::InvalidRawParts);
        };

        Ok(Self {
            ptr,
            allocated: mem::size_of::<T>() != 0,
            _not_sync: PhantomData,
        })
    }
}

impl<T> Drop for RallocBox<T> {
    fn drop(&mut self) {
        // SAFETY: `ptr` points to the initialized value owned by this handle.
        unsafe { ptr::drop_in_place(self.ptr.as_ptr()) };
        if self.allocated {
            // SAFETY: After dropping the value, the allocation is still owned by
            // this handle and must be released exactly once.
            unsafe { ralloc_free(self.ptr.as_ptr().cast::<c_void>()) };
        }
    }
}

impl<T> Deref for RallocBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> DerefMut for RallocBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}
