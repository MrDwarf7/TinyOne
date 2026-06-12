//! Ralloc is a `no_std` malloc-family allocator crate with a narrow C ABI.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unused_must_use)]
#![deny(missing_docs)]

mod arena;
mod backend;
mod block;
mod buffer;
mod page;
mod ralloc;
mod region;
mod sync;

pub use buffer::{RallocBox, RallocBuffer, RallocError};
pub use ralloc::{ralloc_aligned_alloc, ralloc_calloc, ralloc_free, ralloc_malloc, ralloc_realloc};
