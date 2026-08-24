use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InstrumentationSnapshot {
    pub(crate) growth_events: u64,
    pub(crate) bytes_copied: u64,
}

static GROWTH_EVENTS: AtomicU64 = AtomicU64::new(0);
static BYTES_COPIED: AtomicU64 = AtomicU64::new(0);

#[inline]
pub(crate) fn record_growth() {
    GROWTH_EVENTS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn record_bytes_copied(bytes: usize) {
    BYTES_COPIED.fetch_add(bytes as u64, Ordering::Relaxed);
}

pub(crate) fn snapshot() -> InstrumentationSnapshot {
    InstrumentationSnapshot {
        growth_events: GROWTH_EVENTS.load(Ordering::Relaxed),
        bytes_copied: BYTES_COPIED.load(Ordering::Relaxed),
    }
}

pub(crate) fn reset() {
    GROWTH_EVENTS.store(0, Ordering::Relaxed);
    BYTES_COPIED.store(0, Ordering::Relaxed);
}
