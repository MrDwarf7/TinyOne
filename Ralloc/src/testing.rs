//! Feature-gated allocator instrumentation for external test harnesses.

/// A process-wide snapshot of allocator resize activity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InstrumentationSnapshot {
    /// Successful buffer growth operations.
    pub growth_events: u64,
    /// Bytes copied while moving live allocations during growth.
    pub bytes_copied: u64,
}

/// Returns the current process-wide allocator instrumentation counters.
pub fn instrumentation_snapshot() -> InstrumentationSnapshot {
    let snapshot = crate::instrumentation::snapshot();
    InstrumentationSnapshot {
        growth_events: snapshot.growth_events,
        bytes_copied: snapshot.bytes_copied,
    }
}

/// Resets all process-wide allocator instrumentation counters.
///
/// Call this only when the surrounding test harness has exclusive ownership
/// of the process-wide measurement interval.
pub fn reset_instrumentation() {
    crate::instrumentation::reset();
}
