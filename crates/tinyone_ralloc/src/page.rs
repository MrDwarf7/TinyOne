//! Page and virtual-memory concepts for platform backends.

/// Byte size of a hardware or operating-system page.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageSize(usize);

#[allow(dead_code)]
impl PageSize {
    /// Creates a page size from a non-zero power-of-two byte count.
    pub(crate) const fn new(bytes: usize) -> Option<Self> {
        if bytes == 0 || !bytes.is_power_of_two() {
            return None;
        }

        Some(Self(bytes))
    }

    /// Returns the size in bytes.
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// Page-aligned address range used by virtual-memory backends.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageRange {
    start:     usize,
    len:       usize,
    page_size: PageSize,
}

#[allow(dead_code)]
impl PageRange {
    /// Creates a page-aligned range with page-multiple length.
    pub(crate) const fn new(start: usize, len: usize, page_size: PageSize) -> Option<Self> {
        if len == 0 {
            return None;
        }

        let bytes = page_size.get();
        if start & (bytes - 1) != 0 || len & (bytes - 1) != 0 {
            return None;
        }

        Some(Self { start, len, page_size })
    }

    /// Returns the first address in the range.
    pub(crate) const fn start(self) -> usize {
        self.start
    }

    /// Returns the range byte length.
    pub(crate) const fn len(self) -> usize {
        self.len
    }

    /// Returns the page size used to validate the range.
    pub(crate) const fn page_size(self) -> PageSize {
        self.page_size
    }
}

/// Abstract virtual-memory lifecycle operation.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualMemoryOperation {
    /// Reserve a virtual address range.
    Reserve,
    /// Commit physical or backing storage for a reserved range.
    Commit,
    /// Remove backing storage while keeping the reservation.
    Decommit,
    /// Release the reservation.
    Release,
}

/// Interface for platform-specific virtual-memory providers.
#[allow(dead_code)]
pub(crate) trait VirtualMemoryOps {
    /// Performs one virtual-memory lifecycle operation over `range`.
    fn apply(&mut self, operation: VirtualMemoryOperation, range: PageRange) -> bool;
}

#[cfg(test)]
mod tests {
    use super::{PageRange, PageSize, VirtualMemoryOperation};

    #[test]
    fn page_size_requires_power_of_two() {
        assert!(PageSize::new(4096).is_some());
        assert!(PageSize::new(3000).is_none());
        assert!(PageSize::new(0).is_none());
    }

    #[test]
    fn page_range_requires_page_aligned_address_and_length() {
        let page_size = PageSize::new(4096).expect("valid page size");
        let range = PageRange::new(0x8000, 8192, page_size).expect("valid range");

        assert_eq!(range.start(), 0x8000);
        assert_eq!(range.len(), 8192);
        assert_eq!(range.page_size(), page_size);
        assert!(PageRange::new(0x8001, 8192, page_size).is_none());
        assert!(PageRange::new(0x8000, 4097, page_size).is_none());
    }

    #[test]
    fn virtual_memory_operations_cover_mapping_lifecycle() {
        let operations = [
            VirtualMemoryOperation::Reserve,
            VirtualMemoryOperation::Commit,
            VirtualMemoryOperation::Decommit,
            VirtualMemoryOperation::Release,
        ];

        assert_eq!(operations.len(), 4);
    }
}
