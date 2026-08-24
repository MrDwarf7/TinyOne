#[cfg(feature = "testing-hooks")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "testing-hooks")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuntimeCostCounters {
    pub(crate) heap_lock_acquisitions: u64,
    pub(crate) value_encodes: u64,
    pub(crate) value_decodes: u64,
}

#[cfg(feature = "testing-hooks")]
static HEAP_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "testing-hooks")]
static VALUE_ENCODES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "testing-hooks")]
static VALUE_DECODES: AtomicU64 = AtomicU64::new(0);

#[inline]
pub(crate) fn record_heap_lock_acquisition() {
    #[cfg(feature = "testing-hooks")]
    HEAP_LOCK_ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn record_value_encode() {
    #[cfg(feature = "testing-hooks")]
    VALUE_ENCODES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn record_value_decode() {
    #[cfg(feature = "testing-hooks")]
    VALUE_DECODES.fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "testing-hooks")]
pub(crate) fn snapshot() -> RuntimeCostCounters {
    RuntimeCostCounters {
        heap_lock_acquisitions: HEAP_LOCK_ACQUISITIONS.load(Ordering::Relaxed),
        value_encodes: VALUE_ENCODES.load(Ordering::Relaxed),
        value_decodes: VALUE_DECODES.load(Ordering::Relaxed),
    }
}

#[cfg(feature = "testing-hooks")]
pub(crate) fn reset() {
    HEAP_LOCK_ACQUISITIONS.store(0, Ordering::Relaxed);
    VALUE_ENCODES.store(0, Ordering::Relaxed);
    VALUE_DECODES.store(0, Ordering::Relaxed);
}
