//! Arena-local allocation state and block mutation.

use crate::backend::MemoryRegion;
use crate::block::{ALIGNMENT, BlockHeader, HEADER_SIZE, MIN_BLOCK_SIZE, align_up, request_to_block_size};
use crate::region::{self, ArenaSlot};

/// Arena allocation failure or pointer validation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArenaError {
    /// The backend-provided region is not usable as an arena span.
    InvalidRegion,
    /// Region registration failed.
    RegionRegistrationFailed,
    /// No free block can satisfy the request.
    OutOfMemory,
    /// The pointer is not a live allocation in this arena.
    InvalidPointer,
}

/// Single arena backed by one contiguous memory span.
pub(crate) struct Arena {
    start: usize,
    len:   usize,
    stats: ArenaStats,
}

/// Per-arena allocation counters for internal diagnostics.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArenaStats {
    current_allocations:   usize,
    current_payload_bytes: usize,
    peak_payload_bytes:    usize,
}

/// Test-only result of walking an arena's block metadata.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArenaIntegritySnapshot {
    block_count:      usize,
    live_block_count: usize,
}

#[cfg(test)]
#[allow(dead_code)]
impl ArenaIntegritySnapshot {
    /// Returns the number of blocks reached by the arena metadata walk.
    pub(crate) const fn block_count(self) -> usize {
        self.block_count
    }

    /// Returns the number of allocated blocks reached by the arena metadata walk.
    pub(crate) const fn live_block_count(self) -> usize {
        self.live_block_count
    }
}

#[allow(dead_code)]
impl ArenaStats {
    /// Returns the number of live allocations in this arena.
    pub(crate) const fn current_allocations(self) -> usize {
        self.current_allocations
    }

    /// Returns the total usable payload bytes currently live in this arena.
    pub(crate) const fn current_payload_bytes(self) -> usize {
        self.current_payload_bytes
    }

    /// Returns the peak live usable payload bytes observed in this arena.
    pub(crate) const fn peak_payload_bytes(self) -> usize {
        self.peak_payload_bytes
    }

    fn record_allocate(&mut self, payload_bytes: usize) {
        self.current_allocations = self.current_allocations.saturating_add(1);
        self.current_payload_bytes = self.current_payload_bytes.saturating_add(payload_bytes);
        self.peak_payload_bytes = self.peak_payload_bytes.max(self.current_payload_bytes);
    }

    fn record_deallocate(&mut self, payload_bytes: usize) {
        self.current_allocations = self.current_allocations.saturating_sub(1);
        self.current_payload_bytes = self.current_payload_bytes.saturating_sub(payload_bytes);
    }
}

impl Arena {
    /// Initializes an arena over a backend-provided region.
    pub(crate) fn from_region(region: MemoryRegion, slot: ArenaSlot, generation: usize) -> Result<Self, ArenaError> {
        let start = region.as_ptr().addr();
        let len = region.len();

        if start & (ALIGNMENT - 1) != 0 || len < MIN_BLOCK_SIZE || len & (ALIGNMENT - 1) != 0 {
            return Err(ArenaError::InvalidRegion);
        }

        region::register(region::RegionRecord::new(start, len, slot, generation).ok_or(ArenaError::InvalidRegion)?)
            .map_err(|_| ArenaError::RegionRegistrationFailed)?;

        let header = start as *mut BlockHeader;
        // SAFETY: `start` is aligned, `len >= MIN_BLOCK_SIZE`, and this arena
        // owns the newly registered span, so writing the initial free header is
        // the first metadata mutation inside the region.
        unsafe { header.write(BlockHeader::new_free(len)) };

        Ok(Self {
            start,
            len,
            stats: ArenaStats::default(),
        })
    }

    /// Allocates `size` payload bytes from this arena.
    #[allow(dead_code)]
    pub(crate) fn allocate(&mut self, size: usize) -> Result<*mut u8, ArenaError> {
        self.allocate_aligned(size, ALIGNMENT)
    }

    /// Allocates `size` payload bytes with a requested payload alignment.
    pub(crate) fn allocate_aligned(&mut self, size: usize, align: usize) -> Result<*mut u8, ArenaError> {
        if align == 0 || !align.is_power_of_two() {
            return Err(ArenaError::OutOfMemory);
        }
        let align = align.max(ALIGNMENT);
        let needed = request_to_block_size(size).ok_or(ArenaError::OutOfMemory)?;
        let mut current = self.start;
        let end = self.end();

        while current < end {
            let header = current as *mut BlockHeader;
            // SAFETY: `current` walks block starts created by this arena inside
            // `start..end`, so reading the block header is within the span.
            let block = unsafe { &mut *header };
            let block_size = block.size();

            if !block.is_allocated() {
                let Some((allocation_header, available_size)) =
                    Self::aligned_allocation_slot(current, block_size, needed, align)
                else {
                    current = current.checked_add(block_size).ok_or(ArenaError::InvalidRegion)?;
                    continue;
                };

                if allocation_header.addr() != current {
                    let leading_size = allocation_header
                        .addr()
                        .checked_sub(current)
                        .ok_or(ArenaError::InvalidRegion)?;
                    // SAFETY: `leading_size` is an aligned usable block inside
                    // the original free block and ends at `allocation_header`.
                    unsafe { header.write(BlockHeader::new_free(leading_size)) };
                }

                Self::allocate_from_block(allocation_header, available_size, needed);
                // SAFETY: `header` points to a live block header owned by this
                // arena; payload_ptr returns the address immediately after it.
                let allocated = unsafe { &*allocation_header };
                let payload = allocated.payload_ptr();
                self.stats.record_allocate(allocated.size().saturating_sub(HEADER_SIZE));
                return Ok(payload);
            }

            current = current.checked_add(block_size).ok_or(ArenaError::InvalidRegion)?;
        }

        Err(ArenaError::OutOfMemory)
    }

    /// Frees a payload pointer previously returned by this arena.
    pub(crate) fn deallocate(&mut self, payload: *mut u8) -> Result<(), ArenaError> {
        if payload.is_null() {
            return Ok(());
        }

        let payload_addr = payload.addr();
        if payload_addr < self.start || payload_addr >= self.end() {
            return Err(ArenaError::InvalidPointer);
        }

        let header = self.find_live_header(payload)?;
        // SAFETY: `find_live_header` only returns known block starts inside
        // this arena for exact live payload pointers.
        let block = unsafe { &mut *header };
        let payload_bytes = block.size().saturating_sub(HEADER_SIZE);

        block.mark_free();
        self.stats.record_deallocate(payload_bytes);
        self.coalesce(header);

        Ok(())
    }

    /// Returns usable payload bytes for a live allocation.
    pub(crate) fn allocation_size(&self, payload: *mut u8) -> Result<usize, ArenaError> {
        if payload.is_null() {
            return Err(ArenaError::InvalidPointer);
        }

        let payload_addr = payload.addr();
        if payload_addr < self.start || payload_addr >= self.end() {
            return Err(ArenaError::InvalidPointer);
        }

        let header = self.find_live_header(payload)?;
        // SAFETY: `find_live_header` only returns known block starts inside
        // this arena for exact live payload pointers.
        let block = unsafe { &*header };

        block.size().checked_sub(HEADER_SIZE).ok_or(ArenaError::InvalidPointer)
    }

    /// Returns a snapshot of internal arena counters.
    #[allow(dead_code)]
    pub(crate) const fn stats(&self) -> ArenaStats {
        self.stats
    }

    /// Walks block metadata and reports a test-only arena integrity snapshot.
    #[cfg(test)]
    pub(crate) fn integrity_snapshot(&self) -> Result<ArenaIntegritySnapshot, ArenaError> {
        let mut current = self.start;
        let end = self.end();
        let mut block_count = 0usize;
        let mut live_block_count = 0usize;

        while current < end {
            let header = current as *mut BlockHeader;
            // SAFETY: `current` starts at the arena base and advances only by
            // block sizes accepted by `is_valid_block_step`.
            let block = unsafe { &*header };
            let block_size = block.size();
            if !is_valid_block_step(current, block_size, end) {
                return Err(ArenaError::InvalidRegion);
            }

            block_count = block_count.checked_add(1).ok_or(ArenaError::InvalidRegion)?;
            if block.is_allocated() {
                live_block_count = live_block_count.checked_add(1).ok_or(ArenaError::InvalidRegion)?;
            }

            current = current.checked_add(block_size).ok_or(ArenaError::InvalidRegion)?;
        }

        if current != end {
            return Err(ArenaError::InvalidRegion);
        }

        Ok(ArenaIntegritySnapshot {
            block_count,
            live_block_count,
        })
    }

    fn find_live_header(&self, payload: *mut u8) -> Result<*mut BlockHeader, ArenaError> {
        let payload_addr = payload.addr();
        let mut current = self.start;
        let end = self.end();

        while current < end {
            let header = current as *mut BlockHeader;
            // SAFETY: `current` is either the arena start or the next address
            // reached from a previously validated block size.
            let block = unsafe { &*header };
            let block_size = block.size();
            if !is_valid_block_step(current, block_size, end) {
                return Err(ArenaError::InvalidRegion);
            }

            if block.is_allocated() && block.payload_ptr().addr() == payload_addr {
                return Ok(header);
            }

            current = current.checked_add(block_size).ok_or(ArenaError::InvalidRegion)?;
        }

        Err(ArenaError::InvalidPointer)
    }

    fn allocate_from_block(header: *mut BlockHeader, block_size: usize, needed: usize) {
        if let Some((head_size, tail_size)) = BlockHeader::split_sizes(block_size, needed) {
            // SAFETY: `head_size + tail_size == block_size`, and both sizes are
            // aligned usable blocks within the original free block.
            unsafe {
                header.write(BlockHeader::new_free(head_size));
                (*header).mark_allocated();
                header
                    .cast::<u8>()
                    .add(head_size)
                    .cast::<BlockHeader>()
                    .write(BlockHeader::new_free(tail_size));
            }
        } else {
            // SAFETY: `header` points to a valid free block selected by
            // allocate; marking it allocated preserves its existing size.
            unsafe { (*header).mark_allocated() };
        }
    }

    fn aligned_allocation_slot(
        block_start: usize,
        block_size: usize,
        needed: usize,
        align: usize,
    ) -> Option<(*mut BlockHeader, usize)> {
        if block_size < needed {
            return None;
        }

        let block_end = block_start.checked_add(block_size)?;
        let mut header_addr = align_up(block_start.checked_add(HEADER_SIZE)?, align)?.checked_sub(HEADER_SIZE)?;

        if header_addr < block_start {
            return None;
        }

        let mut leading_size = header_addr.checked_sub(block_start)?;
        if leading_size > 0 && leading_size < MIN_BLOCK_SIZE {
            header_addr = header_addr.checked_add(align)?;
            leading_size = header_addr.checked_sub(block_start)?;
        }

        if leading_size > 0 && leading_size & (ALIGNMENT - 1) != 0 {
            return None;
        }
        if leading_size > 0 && leading_size < MIN_BLOCK_SIZE {
            return None;
        }

        let allocation_end = header_addr.checked_add(needed)?;
        if allocation_end > block_end {
            return None;
        }

        let available_size = block_end.checked_sub(header_addr)?;
        Some((header_addr as *mut BlockHeader, available_size))
    }

    fn coalesce(&mut self, header: *mut BlockHeader) {
        let header = self.coalesce_with_next(header);

        if let Some(previous) = self.previous_block(header) {
            // SAFETY: `previous` is a valid block start found by scanning this
            // arena, and `header` is the next block being freed.
            let previous_block = unsafe { &mut *previous };
            // SAFETY: `header` is a valid block pointer in this arena.
            let current_block = unsafe { &mut *header };

            if !previous_block.is_allocated()
                && let Some(size) = BlockHeader::coalesced_size(previous_block.size(), current_block.size())
            {
                // SAFETY: The two free blocks are adjacent by construction
                // from `previous_block`; replacing the left header expands
                // it over the right block.
                unsafe { previous.write(BlockHeader::new_free(size)) };
                let _ = self.coalesce_with_next(previous);
            }
        }
    }

    fn coalesce_with_next(&self, header: *mut BlockHeader) -> *mut BlockHeader {
        // SAFETY: `header` points to a block in this arena.
        let block = unsafe { &mut *header };
        let next_addr = header.addr() + block.size();

        if next_addr >= self.end() {
            return header;
        }

        let next = next_addr as *mut BlockHeader;
        // SAFETY: `next_addr` is inside the arena and follows the current
        // block size, so it is the next block header.
        let next_block = unsafe { &mut *next };
        if next_block.is_allocated() {
            return header;
        }

        if let Some(size) = BlockHeader::coalesced_size(block.size(), next_block.size()) {
            // SAFETY: The current and next blocks are adjacent free blocks.
            unsafe { header.write(BlockHeader::new_free(size)) };
        }

        header
    }

    fn previous_block(&self, header: *mut BlockHeader) -> Option<*mut BlockHeader> {
        let mut current = self.start;
        let target = header.addr();
        let mut previous = None;

        while current < target {
            let current_header = current as *mut BlockHeader;
            previous = Some(current_header);
            // SAFETY: `current` walks known block starts before `target`.
            let size = unsafe { (*current_header).size() };
            current = current.checked_add(size)?;
        }

        (current == target).then_some(previous?)
    }

    const fn end(&self) -> usize {
        self.start + self.len
    }
}

fn is_valid_block_step(current: usize, block_size: usize, end: usize) -> bool {
    block_size >= MIN_BLOCK_SIZE
        && block_size & (ALIGNMENT - 1) == 0
        && current.checked_add(block_size).is_some_and(|next| next <= end)
}

impl Drop for Arena {
    fn drop(&mut self) {
        let _ = region::remove(self.start, self.len);
    }
}

#[cfg(test)]
mod tests {
    use super::Arena;
    use crate::backend::{FixedRegionBackend, MemoryBackend, RegionRequest};
    use crate::block::BlockHeader;
    use crate::region::{self, ArenaSlot};

    #[repr(align(64))]
    struct AlignedStorage([u8; 512]);

    fn test_arena(storage: &mut AlignedStorage) -> Arena {
        region::clear_for_tests();
        let mut backend = FixedRegionBackend::new(storage.0.as_mut_ptr(), storage.0.len());
        let region = backend
            .allocate_region(RegionRequest::new(storage.0.len(), 64).unwrap())
            .expect("test region should fit");

        Arena::from_region(region, ArenaSlot::new(0), 0).expect("arena should initialize")
    }

    #[test]
    fn arena_allocates_and_reuses_freed_blocks() {
        let _guard = region::TEST_LOCK.lock();
        let mut storage = AlignedStorage([0; 512]);
        let mut arena = test_arena(&mut storage);

        let first = arena.allocate(24).expect("allocation should succeed");
        arena.deallocate(first).expect("free should succeed");
        let second = arena.allocate(24).expect("allocation should reuse space");

        assert_eq!(first, second);
    }

    #[test]
    fn arena_coalesces_adjacent_free_blocks() {
        let _guard = region::TEST_LOCK.lock();
        let mut storage = AlignedStorage([0; 512]);
        let mut arena = test_arena(&mut storage);

        let first = arena.allocate(96).expect("first allocation should succeed");
        let second = arena.allocate(96).expect("second allocation should succeed");

        arena.deallocate(first).expect("first free should succeed");
        arena.deallocate(second).expect("second free should succeed");

        let combined = arena
            .allocate(208)
            .expect("coalesced free blocks should satisfy larger request");
        assert_eq!(combined, first);
    }

    #[test]
    fn arena_rejects_in_region_non_payload_pointers() {
        let _guard = region::TEST_LOCK.lock();
        let mut storage = AlignedStorage([0; 512]);
        let mut arena = test_arena(&mut storage);

        let allocation = arena.allocate(64).expect("allocation should succeed");
        let non_payload = allocation.wrapping_sub(1);

        assert_eq!(arena.deallocate(non_payload), Err(super::ArenaError::InvalidPointer));
        assert_eq!(arena.allocation_size(non_payload), Err(super::ArenaError::InvalidPointer));

        arena
            .deallocate(allocation)
            .expect("original allocation should remain live");
    }

    #[test]
    fn arena_stats_track_current_and_peak_payload_bytes() {
        let _guard = region::TEST_LOCK.lock();
        let mut storage = AlignedStorage([0; 512]);
        let mut arena = test_arena(&mut storage);

        assert_eq!(arena.stats().current_allocations(), 0);
        assert_eq!(arena.stats().current_payload_bytes(), 0);
        assert_eq!(arena.stats().peak_payload_bytes(), 0);

        let first = arena.allocate(24).expect("first allocation should succeed");
        let first_size = arena
            .allocation_size(first)
            .expect("first allocation should have a size");
        assert_eq!(arena.stats().current_allocations(), 1);
        assert_eq!(arena.stats().current_payload_bytes(), first_size);
        assert_eq!(arena.stats().peak_payload_bytes(), first_size);

        let second = arena.allocate(64).expect("second allocation should succeed");
        let second_size = arena
            .allocation_size(second)
            .expect("second allocation should have a size");
        assert_eq!(arena.stats().current_allocations(), 2);
        assert_eq!(arena.stats().current_payload_bytes(), first_size + second_size);
        assert_eq!(arena.stats().peak_payload_bytes(), first_size + second_size);

        arena.deallocate(first).expect("first free should succeed");
        assert_eq!(arena.stats().current_allocations(), 1);
        assert_eq!(arena.stats().current_payload_bytes(), second_size);
        assert_eq!(arena.stats().peak_payload_bytes(), first_size + second_size);

        arena.deallocate(second).expect("second free should succeed");
        assert_eq!(arena.stats().current_allocations(), 0);
        assert_eq!(arena.stats().current_payload_bytes(), 0);
        assert_eq!(arena.stats().peak_payload_bytes(), first_size + second_size);
    }

    #[test]
    fn arena_allocate_aligned_returns_requested_payload_alignment() {
        let _guard = region::TEST_LOCK.lock();
        let mut storage = AlignedStorage([0; 512]);
        let mut arena = test_arena(&mut storage);

        let payload = arena
            .allocate_aligned(24, 64)
            .expect("aligned allocation should succeed");

        assert_eq!(payload.addr() % 64, 0);
        assert!(arena.allocation_size(payload).is_ok());

        arena.deallocate(payload).expect("aligned allocation should free");
    }

    #[test]
    fn integrity_snapshot_rejects_invalid_block_step() {
        let _guard = region::TEST_LOCK.lock();
        let mut storage = AlignedStorage([0; 512]);
        let arena = test_arena(&mut storage);

        let first_header = arena.start as *mut BlockHeader;
        // SAFETY: This test intentionally corrupts allocator metadata inside
        // its private arena span to verify the integrity hook catches it.
        unsafe { first_header.write(BlockHeader::new_free(super::ALIGNMENT)) };

        assert_eq!(arena.integrity_snapshot(), Err(super::ArenaError::InvalidRegion));
    }
}
