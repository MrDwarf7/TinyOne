use crate::RuntimeValue;

// These are temporary safety caps, NOT design philosophy. TinyOne does not
// intend to limit users. Each constant below exists because the allocator
// backend (Ralloc integration, GAT, IRC, FrameRegion) is not yet
// production-grade. As those systems mature, these become configurable or
// are removed entirely. See `phase_2_allocator.md` for the removal roadmap.
//
// MAX_CALL_DEPTH is the one exception: it is load-bearing until the shadow
// stack / FrameRegion system (phase_2_allocator.md [REGION-1]) is built,
// because the heap shutdown walk assumes bounded recursion. All others are
// pure safety caps with no structural dependency.
pub(crate) const MAX_CALL_DEPTH: usize = 16;
pub(crate) const MAX_HEAP_OBJECTS: usize = 1_000_000;
pub(crate) const MAX_HEAP_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_ARRAY_LENGTH: usize = 65_536;
pub(crate) const MAX_BUFFER_BYTES: usize = 1024 * 1024;
pub(crate) const VALUE_BYTES: usize = std::mem::size_of::<RuntimeValue>();

/// Per-program VM policy persisted in bytecode artifacts.
///
/// Limits can only tighten the runtime's hard safety ceiling. Keeping the
/// ceiling in the runtime avoids a project configuration accidentally turning
/// an embedding application into an unbounded executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VmSettings {
    pub(crate) max_call_depth: usize,
}

impl VmSettings {
    pub(crate) const fn defaulted() -> Self {
        Self {
            max_call_depth: MAX_CALL_DEPTH,
        }
    }

    pub(crate) fn with_max_call_depth(max_call_depth: usize) -> crate::Result<Self> {
        if max_call_depth == 0 || max_call_depth > MAX_CALL_DEPTH {
            return Err(crate::TinyOneError::compile(format!(
                "VM max_call_depth must be between 1 and {MAX_CALL_DEPTH}"
            )));
        }
        Ok(Self { max_call_depth })
    }
}

impl Default for VmSettings {
    fn default() -> Self {
        Self::defaulted()
    }
}
