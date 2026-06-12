//! Block metadata and checked size arithmetic for allocator-owned regions.

use core::mem;

pub const ALIGNMENT: usize = mem::size_of::<usize>() * 2;
const ALLOCATED_FLAG: usize = 1;
const SIZE_MASK: usize = !ALLOCATED_FLAG;

pub(crate) const HEADER_SIZE: usize = align_up_const(mem::size_of::<BlockHeader>(), ALIGNMENT);
pub(crate) const MIN_BLOCK_SIZE: usize = HEADER_SIZE + ALIGNMENT;

#[repr(C)]
pub(crate) struct BlockHeader {
    size_and_flags: usize,
}

impl BlockHeader {
    pub(crate) const fn new_free(size: usize) -> Self {
        Self {
            size_and_flags: size & SIZE_MASK,
        }
    }

    pub(crate) const fn size(&self) -> usize {
        self.size_and_flags & SIZE_MASK
    }

    pub(crate) fn mark_allocated(&mut self) {
        self.size_and_flags |= ALLOCATED_FLAG;
    }

    pub(crate) fn mark_free(&mut self) {
        self.size_and_flags &= SIZE_MASK;
    }

    pub(crate) const fn is_allocated(&self) -> bool {
        self.size_and_flags & ALLOCATED_FLAG != 0
    }

    pub(crate) fn payload_ptr(&self) -> *mut u8 {
        let header = core::ptr::from_ref(self).cast::<u8>().cast_mut();
        // SAFETY: Payload starts HEADER_SIZE bytes after a valid block header.
        unsafe { header.add(HEADER_SIZE) }
    }

    pub(crate) fn split_sizes(block_size: usize, requested_size: usize) -> Option<(usize, usize)> {
        if requested_size > block_size {
            return None;
        }

        let remainder = block_size.checked_sub(requested_size)?;
        if remainder < MIN_BLOCK_SIZE {
            return None;
        }
        if !is_aligned(requested_size) || !is_aligned(remainder) {
            return None;
        }

        Some((requested_size, remainder))
    }

    pub(crate) fn coalesced_size(left_size: usize, right_size: usize) -> Option<usize> {
        let total = left_size.checked_add(right_size)?;
        is_aligned(total).then_some(total)
    }
}

pub(crate) const fn align_up_const(value: usize, align: usize) -> usize {
    (value + (align - 1)) & !(align - 1)
}

pub(crate) fn align_up(value: usize, align: usize) -> Option<usize> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }

    value.checked_add(align - 1).map(|sum| sum & !(align - 1))
}

pub(crate) fn request_to_block_size(request: usize) -> Option<usize> {
    let payload_size = align_up(request.max(1), ALIGNMENT)?;
    let block_size = HEADER_SIZE.checked_add(payload_size)?;

    Some(block_size.max(MIN_BLOCK_SIZE))
}

pub(crate) const fn is_aligned(value: usize) -> bool {
    value & (ALIGNMENT - 1) == 0
}

#[cfg(test)]
mod tests {
    use super::{BlockHeader, HEADER_SIZE, MIN_BLOCK_SIZE, align_up, request_to_block_size};

    #[test]
    fn request_size_is_aligned_and_includes_header() {
        let size = request_to_block_size(1).expect("one-byte allocation should fit");

        assert_eq!(size % super::ALIGNMENT, 0);
        assert!(size >= MIN_BLOCK_SIZE);
        assert!(size > HEADER_SIZE);
    }

    #[test]
    fn request_size_rejects_overflow() {
        assert_eq!(request_to_block_size(usize::MAX), None);
    }

    #[test]
    fn split_keeps_two_usable_aligned_blocks() {
        let original = request_to_block_size(128).expect("request should fit");
        let requested = request_to_block_size(32).expect("request should fit");

        let (head, tail) =
            BlockHeader::split_sizes(original, requested).expect("large block should split into two usable blocks");

        assert_eq!(head, requested);
        assert_eq!(head + tail, original);
        assert_eq!(tail % super::ALIGNMENT, 0);
        assert!(tail >= MIN_BLOCK_SIZE);
    }

    #[test]
    fn adjacent_free_blocks_coalesce_by_size() {
        let left = request_to_block_size(24).expect("request should fit");
        let right = request_to_block_size(40).expect("request should fit");

        assert_eq!(BlockHeader::coalesced_size(left, right), Some(left + right));
    }

    #[test]
    fn align_up_rejects_overflow() {
        assert_eq!(align_up(usize::MAX, super::ALIGNMENT), None);
    }
}
