//! Ralloc is a `no_std` malloc-family allocator crate with a narrow C ABI.
//!
//! Can be built as:
//! - `rlib` (default) — for use as a Rust dependency
//! - `cdylib` — for dynamic linking from C/C++
//!
//! Build with `--features cdylib` to produce the C dynamic library.

#![cfg_attr(not(feature = "rlib"), no_std)]
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

// Re-export C ABI functions for Rust consumers
pub use ralloc::{ralloc_aligned_alloc, ralloc_calloc, ralloc_free, ralloc_malloc, ralloc_realloc};

// Panic handler for no_std builds (cdylib, i.e. when rlib is NOT active)
#[cfg(not(feature = "rlib"))]
use core::panic::PanicInfo;

#[cfg(not(feature = "rlib"))]
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {}
}
