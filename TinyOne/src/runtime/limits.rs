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
