extern crate std;

use ralloc::{RallocBox, RallocBuffer};
use static_assertions::{assert_impl_all, assert_not_impl_all};

// RallocBuffer: Send + !Sync
assert_impl_all!(RallocBuffer: Send);
assert_not_impl_all!(RallocBuffer: Sync);

// RallocBox<T: Send>: Send + !Sync
assert_impl_all!(RallocBox<usize>: Send);
assert_not_impl_all!(RallocBox<usize>: Sync);

// RallocBox<T: !Send> must itself be !Send
assert_not_impl_all!(RallocBox<*const u8>: Send);
