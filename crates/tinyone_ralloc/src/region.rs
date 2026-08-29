//! Address-region ownership registry for routing pointers to arenas.
//!
//! Synchronization policy: the global registry is protected by its own
//! `SpinLock`, and every public helper below acquires that lock for exactly one
//! lookup, insert, removal, clear, or snapshot operation. The registry lock
//! protects only the sorted region table; it is not held across arena block
//! mutation. Allocator paths first use the registry to identify an arena owner,
//! drop the registry guard, and then acquire the owning arena lock. Region
//! registration/removal is similarly a short registry-only update performed
//! during arena lifetime changes, not during steady-state block mutation.

use crate::sync::SpinLock;

const REGION_CAPACITY: usize = 64;

/// Identifier for an arena slot in the arena manager.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArenaSlot(usize);

impl ArenaSlot {
    /// Creates an arena slot identifier.
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Returns the numeric slot index.
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// Registered backend-provided span and its owning arena.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegionRecord {
    start:      usize,
    len:        usize,
    owner:      ArenaSlot,
    generation: usize,
}

impl RegionRecord {
    /// Creates a non-empty region record if `start + len` does not overflow.
    pub(crate) const fn new(start: usize, len: usize, owner: ArenaSlot, generation: usize) -> Option<Self> {
        if len == 0 || start.checked_add(len).is_none() {
            return None;
        }

        Some(Self {
            start,
            len,
            owner,
            generation,
        })
    }

    /// Returns the first address in the region.
    pub(crate) const fn start(self) -> usize {
        self.start
    }

    /// Returns one byte past the region.
    pub(crate) const fn end(self) -> usize {
        self.start + self.len
    }

    /// Returns the region byte length.
    pub(crate) const fn len(self) -> usize {
        self.len
    }

    /// Returns the owning arena slot.
    pub(crate) const fn owner(self) -> ArenaSlot {
        self.owner
    }

    /// Returns the publication generation.
    #[allow(dead_code)]
    pub(crate) const fn generation(self) -> usize {
        self.generation
    }
}

/// Region registry operation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegionError {
    /// The fixed-capacity registry is full.
    Full,
    /// The requested range overlaps an existing region.
    OverlapsExisting,
    /// No exact matching region was found.
    NotFound,
}

/// Fixed-capacity region table sorted by start address.
pub(crate) struct RegionRegistry {
    records: [Option<RegionRecord>; REGION_CAPACITY],
    len:     usize,
}

impl RegionRegistry {
    /// Creates an empty registry.
    pub(crate) const fn new() -> Self {
        Self {
            records: [None; REGION_CAPACITY],
            len:     0,
        }
    }

    /// Registers a region while preserving sorted order.
    pub(crate) fn register(&mut self, record: RegionRecord) -> Result<(), RegionError> {
        if self.len == REGION_CAPACITY {
            return Err(RegionError::Full);
        }

        let index = self.lower_bound(record.start());
        if index > 0 {
            let previous = self.records[index - 1].expect("entries before len are populated");
            if previous.end() > record.start() {
                return Err(RegionError::OverlapsExisting);
            }
        }
        if index < self.len {
            let next = self.records[index].expect("entries before len are populated");
            if record.end() > next.start() {
                return Err(RegionError::OverlapsExisting);
            }
        }

        for slot in (index..self.len).rev() {
            self.records[slot + 1] = self.records[slot];
        }
        self.records[index] = Some(record);
        self.len += 1;

        Ok(())
    }

    /// Finds the region containing `address`.
    pub(crate) fn lookup(&self, address: usize) -> Option<RegionRecord> {
        let index = self.lower_bound_after(address)?;
        let record = self.records[index].expect("entries before len are populated");

        (address < record.end()).then_some(record)
    }

    /// Removes an exact registered range.
    pub(crate) fn remove(&mut self, start: usize, len: usize) -> Result<RegionRecord, RegionError> {
        let index = self.lower_bound(start);
        if index == self.len {
            return Err(RegionError::NotFound);
        }

        let record = self.records[index].expect("entries before len are populated");
        if record.start() != start || record.len() != len {
            return Err(RegionError::NotFound);
        }

        for slot in index..(self.len - 1) {
            self.records[slot] = self.records[slot + 1];
        }
        self.len -= 1;
        self.records[self.len] = None;

        Ok(record)
    }

    fn lower_bound(&self, start: usize) -> usize {
        let mut low = 0;
        let mut high = self.len;

        while low < high {
            let mid = low + ((high - low) / 2);
            let record = self.records[mid].expect("entries before len are populated");

            if record.start() < start {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        low
    }

    fn lower_bound_after(&self, address: usize) -> Option<usize> {
        let mut low = 0;
        let mut high = self.len;

        while low < high {
            let mid = low + ((high - low) / 2);
            let record = self.records[mid].expect("entries before len are populated");

            if record.start() <= address {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        low.checked_sub(1)
    }
}

static REGISTRY: SpinLock<RegionRegistry> = SpinLock::new(RegionRegistry::new());

#[cfg(test)]
pub(crate) static TEST_LOCK: SpinLock<()> = SpinLock::new(());

/// Registers a region in the global registry.
pub(crate) fn register(record: RegionRecord) -> Result<(), RegionError> {
    REGISTRY.lock().register(record)
}

/// Looks up an address in the global registry.
pub(crate) fn lookup(address: usize) -> Option<RegionRecord> {
    REGISTRY.lock().lookup(address)
}

/// Removes an exact region from the global registry.
pub(crate) fn remove(start: usize, len: usize) -> Result<RegionRecord, RegionError> {
    REGISTRY.lock().remove(start, len)
}

#[cfg(test)]
pub(crate) fn clear_for_tests() {
    *REGISTRY.lock() = RegionRegistry::new();
}

#[cfg(test)]
pub(crate) fn snapshot_lookup_for_tests(address: usize) -> Option<RegionRecord> {
    REGISTRY.lock().lookup(address)
}

#[cfg(test)]
pub(crate) fn snapshots_for_tests() -> [Option<RegionRecord>; REGION_CAPACITY] {
    REGISTRY.lock().records
}

#[cfg(test)]
pub(crate) fn len_for_tests() -> usize {
    REGISTRY.lock().len
}

#[cfg(test)]
mod tests {
    use super::{ArenaSlot, RegionRecord, RegionRegistry, clear_for_tests, lookup, register, remove};

    #[test]
    fn registry_finds_valid_pointers_and_rejects_boundaries() {
        let mut registry = RegionRegistry::new();
        let owner = ArenaSlot::new(2);

        registry
            .register(RegionRecord::new(0x1000, 0x100, owner, 7).expect("valid region"))
            .expect("region should register");

        assert_eq!(registry.lookup(0x1000).map(|record| record.owner()), Some(owner));
        assert_eq!(registry.lookup(0x10ff).map(|record| record.owner()), Some(owner));
        assert!(registry.lookup(0x1100).is_none());
        assert!(registry.lookup(0x0fff).is_none());
    }

    #[test]
    fn registry_rejects_overlapping_regions() {
        let mut registry = RegionRegistry::new();

        registry
            .register(RegionRecord::new(0x1000, 0x100, ArenaSlot::new(0), 0).unwrap())
            .expect("first region should register");

        assert_eq!(
            registry.register(RegionRecord::new(0x1080, 0x80, ArenaSlot::new(1), 0).unwrap()),
            Err(super::RegionError::OverlapsExisting)
        );
    }

    #[test]
    fn registry_removes_only_exact_ranges() {
        let mut registry = RegionRegistry::new();
        let region = RegionRecord::new(0x2000, 0x200, ArenaSlot::new(1), 3).unwrap();

        registry.register(region).expect("region should register");

        assert_eq!(registry.remove(0x2000, 0x100), Err(super::RegionError::NotFound));
        assert_eq!(registry.lookup(0x2000), Some(region));

        registry.remove(0x2000, 0x200).expect("exact range should remove");
        assert!(registry.lookup(0x2000).is_none());
    }

    #[test]
    fn global_registry_routes_registered_addresses() {
        let _guard = super::TEST_LOCK.lock();
        super::clear_for_tests();

        let region = RegionRecord::new(0x3000, 0x100, ArenaSlot::new(4), 9).unwrap();
        register(region).expect("global region should register");

        assert_eq!(lookup(0x3070).map(|record| record.owner()), Some(ArenaSlot::new(4)));
        assert!(lookup(0x4000).is_none());

        remove(0x3000, 0x100).expect("global region should remove");
        assert!(lookup(0x3070).is_none());
    }

    #[test]
    fn test_hook_snapshots_exact_owner_lookup_for_pointer() {
        let _guard = super::TEST_LOCK.lock();
        clear_for_tests();

        let region = RegionRecord::new(0x5000, 0x100, ArenaSlot::new(3), 11).unwrap();
        register(region).expect("global region should register");

        assert_eq!(super::snapshot_lookup_for_tests(0x5040), Some(region));
        assert_eq!(super::snapshot_lookup_for_tests(0x5100), None);
    }

    #[test]
    fn test_hook_snapshots_registry_entries_for_counting() {
        let _guard = super::TEST_LOCK.lock();
        clear_for_tests();

        let first = RegionRecord::new(0x1000, 0x100, ArenaSlot::new(0), 1).unwrap();
        let second = RegionRecord::new(0x3000, 0x200, ArenaSlot::new(1), 2).unwrap();
        register(second).expect("second region should register");
        register(first).expect("first region should register");

        assert_eq!(super::len_for_tests(), 2);
        let entries = super::snapshots_for_tests();
        assert_eq!(entries[0], Some(first));
        assert_eq!(entries[1], Some(second));
        assert!(entries[2].is_none());
    }
}
