//! Memory-source abstractions for allocator-owned regions.

use core::ptr::NonNull;

/// Request for a backend-provided memory region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegionRequest {
    size:  usize,
    align: usize,
}

impl RegionRequest {
    /// Creates a region request with a non-zero size and power-of-two alignment.
    pub(crate) const fn new(size: usize, align: usize) -> Option<Self> {
        if size == 0 || align == 0 || !align.is_power_of_two() {
            return None;
        }

        Some(Self { size, align })
    }

    /// Returns the requested byte length.
    pub(crate) const fn size(self) -> usize {
        self.size
    }

    /// Returns the requested alignment.
    pub(crate) const fn align(self) -> usize {
        self.align
    }
}

/// Backend-owned memory span handed to an arena.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MemoryRegion {
    ptr: NonNull<u8>,
    len: usize,
}

impl MemoryRegion {
    /// Creates a memory region from a non-null pointer and byte length.
    pub(crate) const fn new(ptr: NonNull<u8>, len: usize) -> Self {
        Self { ptr, len }
    }

    /// Returns the first byte of the region.
    pub(crate) const fn as_ptr(self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Returns the region byte length.
    pub(crate) const fn len(self) -> usize {
        self.len
    }
}

/// Interface implemented by allocator memory sources.
pub(crate) trait MemoryBackend {
    /// Allocates one region matching `request`, or returns `None` on OOM.
    fn allocate_region(&mut self, request: RegionRequest) -> Option<MemoryRegion>;
}

/// Deterministic monotonic backend backed by one caller-provided byte span.
pub(crate) struct FixedRegionBackend {
    base:   NonNull<u8>,
    len:    usize,
    offset: usize,
}

impl FixedRegionBackend {
    /// Creates a fixed-region backend over `ptr..ptr + len`.
    pub(crate) fn new(ptr: *mut u8, len: usize) -> Self {
        let base = NonNull::new(ptr).unwrap_or(NonNull::dangling());
        Self { base, len, offset: 0 }
    }
}

impl MemoryBackend for FixedRegionBackend {
    fn allocate_region(&mut self, request: RegionRequest) -> Option<MemoryRegion> {
        let current = self.base.as_ptr().addr().checked_add(self.offset)?;
        let aligned = align_up(current, request.align())?;
        let aligned_offset = aligned.checked_sub(self.base.as_ptr().addr())?;
        let end = aligned_offset.checked_add(request.size())?;

        if end > self.len {
            return None;
        }

        self.offset = end;

        // SAFETY: `aligned_offset <= end <= self.len`, so the pointer remains
        // within the caller-provided span managed by this backend.
        let ptr = unsafe { self.base.as_ptr().add(aligned_offset) };
        NonNull::new(ptr).map(|ptr| MemoryRegion::new(ptr, request.size()))
    }
}

fn align_up(value: usize, align: usize) -> Option<usize> {
    debug_assert!(align.is_power_of_two());
    value.checked_add(align - 1).map(|sum| sum & !(align - 1))
}

#[cfg(test)]
mod tests {
    use super::{FixedRegionBackend, MemoryBackend, RegionRequest};

    #[repr(align(64))]
    struct AlignedStorage([u8; 128]);

    #[test]
    fn fixed_region_backend_returns_aligned_regions() {
        let mut storage = AlignedStorage([0; 128]);
        let mut backend = FixedRegionBackend::new(storage.0.as_mut_ptr(), storage.0.len());

        let region = backend
            .allocate_region(RegionRequest::new(32, 32).expect("request should be valid"))
            .expect("region should fit");

        assert_eq!(region.len(), 32);
        assert_eq!(region.as_ptr().addr() % 32, 0);
    }

    #[test]
    fn fixed_region_backend_reports_out_of_memory() {
        let mut storage = [0u8; 64];
        let mut backend = FixedRegionBackend::new(storage.as_mut_ptr(), storage.len());

        assert!(
            backend
                .allocate_region(RegionRequest::new(48, 8).expect("request should be valid"))
                .is_some()
        );
        assert!(
            backend
                .allocate_region(RegionRequest::new(32, 8).expect("request should be valid"))
                .is_none()
        );
    }
}
