//! Small `no_std` synchronization primitives used by allocator internals.
//!
//! Lock-ordering policy: allocator code may hold an arena lock while mutating
//! arena-local block metadata, but it must not keep the global region-registry
//! lock across arena mutation. Registry operations are short lookup/update
//! critical sections, and test-only metrics must never be exported through the
//! C ABI.

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
#[cfg(test)]
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[allow(dead_code)]
const UNINITIALIZED: u8 = 0;
#[allow(dead_code)]
const INITIALIZING: u8 = 1;
#[allow(dead_code)]
const INITIALIZED: u8 = 2;

/// A minimal non-poisoning spin lock for short allocator critical sections.
///
/// The lock uses acquire/release ordering around the atomic lock bit. It does
/// not track panics and therefore never poisons; if a guard is dropped during
/// unwinding, the lock is released the same way as any other drop.
pub(crate) struct SpinLock<T> {
    locked:  AtomicBool,
    #[cfg(test)]
    metrics: SpinLockMetrics,
    value:   UnsafeCell<T>,
}

// SAFETY: `SpinLock` serializes mutable access with the atomic lock bit. A
// shared reference can be sent to another thread when the protected value can
// be sent across threads.
unsafe impl<T: Send> Sync for SpinLock<T> {}

// SAFETY: Moving the lock transfers ownership of the contained value. Sending
// it is sound when the contained value itself can be sent across threads.
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Creates a lock containing `value`.
    pub(crate) const fn new(value: T) -> Self {
        Self {
            locked:               AtomicBool::new(false),
            #[cfg(test)]
            metrics:              SpinLockMetrics::new(),
            value:                UnsafeCell::new(value),
        }
    }

    /// Acquires the lock, spinning until it becomes available.
    pub(crate) fn lock(&self) -> SpinLockGuard<'_, T> {
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }

            spin_loop();
        }
    }

    /// Attempts to acquire the lock without waiting.
    pub(crate) fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        #[cfg(test)]
        self.metrics.record_try_lock_attempt();

        match self.locked.compare_exchange(
            false,
            true,
            // Acquire pairs with `unlock`'s Release store so the new guard
            // observes all writes made by the previous guard.
            Ordering::Acquire,
            // Failure does not acquire the guard or read protected data, so
            // no synchronization with the previous owner is needed.
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                Some(SpinLockGuard {
                    lock:      self,
                    _not_send: PhantomData,
                })
            }
            Err(_) => {
                #[cfg(test)]
                self.metrics.record_failed_try_lock_attempt();

                None
            }
        }
    }

    #[cfg(test)]
    fn contention_metrics(&self) -> SpinLockContentionMetrics {
        self.metrics.snapshot()
    }

    fn unlock(&self) {
        self.locked.store(
            false,
            // Release publishes protected writes before another thread can
            // acquire the lock with the compare_exchange Acquire success order.
            Ordering::Release,
        );
    }
}

#[cfg(test)]
struct SpinLockMetrics {
    try_lock_attempts:        AtomicUsize,
    failed_try_lock_attempts: AtomicUsize,
}

#[cfg(test)]
impl SpinLockMetrics {
    const fn new() -> Self {
        Self {
            try_lock_attempts:        AtomicUsize::new(0),
            failed_try_lock_attempts: AtomicUsize::new(0),
        }
    }

    fn record_try_lock_attempt(&self) {
        self.try_lock_attempts.fetch_add(
            1,
            // Metrics are diagnostic counters only; they do not order access to
            // protected allocator state.
            Ordering::Relaxed,
        );
    }

    fn record_failed_try_lock_attempt(&self) {
        self.failed_try_lock_attempts.fetch_add(
            1,
            // Failed-attempt metrics are diagnostic counters only and do not
            // participate in synchronization.
            Ordering::Relaxed,
        );
    }

    fn snapshot(&self) -> SpinLockContentionMetrics {
        SpinLockContentionMetrics {
            try_lock_attempts:        self.try_lock_attempts.load(
                // Snapshot reads are diagnostic only; approximate concurrent
                // values are acceptable for test pressure metrics.
                Ordering::Relaxed,
            ),
            failed_try_lock_attempts: self.failed_try_lock_attempts.load(
                // Snapshot reads are diagnostic only and do not protect data.
                Ordering::Relaxed,
            ),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SpinLockContentionMetrics {
    try_lock_attempts:        usize,
    failed_try_lock_attempts: usize,
}

#[cfg(test)]
impl SpinLockContentionMetrics {
    const fn try_lock_attempts(self) -> usize {
        self.try_lock_attempts
    }

    const fn failed_try_lock_attempts(self) -> usize {
        self.failed_try_lock_attempts
    }

    const fn observed_contention(self) -> bool {
        self.failed_try_lock_attempts > 0
    }
}

/// Guard returned from `SpinLock`.
pub(crate) struct SpinLockGuard<'a, T> {
    lock:      &'a SpinLock<T>,
    _not_send: PhantomData<*mut ()>,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: Holding a guard proves the lock bit was acquired, so no
        // mutable reference can be created by another guard until this one is
        // dropped.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: `SpinLockGuard` is unique for a held lock, and `&mut self`
        // prevents this guard from producing two simultaneous mutable borrows.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}

/// Atomic initialization state for shared allocator startup paths.
#[allow(dead_code)]
pub(crate) struct InitializationState {
    state: AtomicU8,
}

#[allow(dead_code)]
impl InitializationState {
    /// Creates an uninitialized state.
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicU8::new(UNINITIALIZED),
        }
    }

    /// Attempts to claim initialization responsibility.
    pub(crate) fn try_begin_initialization(&self) -> bool {
        self.state
            .compare_exchange(
                UNINITIALIZED,
                INITIALIZING,
                // The winning initializer publishes its claim and observes any
                // previous initialization state transition.
                Ordering::AcqRel,
                // A losing initializer observes the current state before
                // deciding whether someone else is initializing or initialized.
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Publishes that initialization has completed.
    pub(crate) fn mark_initialized(&self) {
        self.state.store(
            INITIALIZED,
            // Release publishes initialization writes before observers load the
            // initialized state with Acquire.
            Ordering::Release,
        );
    }

    /// Returns whether initialization has completed.
    pub(crate) fn is_initialized(&self) -> bool {
        self.state.load(
            // Acquire pairs with `mark_initialized`'s Release store so callers
            // that see initialized also see the initialized data.
            Ordering::Acquire,
        ) == INITIALIZED
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::sync::Arc;
    use std::thread;

    use super::{InitializationState, SpinLock};

    #[test]
    fn lock_releases_when_guard_is_dropped() {
        let lock = SpinLock::new(7usize);

        {
            let mut guard = lock.lock();
            *guard += 1;
            assert!(lock.try_lock().is_none());
        }

        let guard = lock.try_lock().expect("lock should be available after drop");
        assert_eq!(*guard, 8);
    }

    #[test]
    fn lock_protects_mutation_across_threads() {
        let lock = Arc::new(SpinLock::new(0usize));
        let mut workers = std::vec::Vec::new();

        for _ in 0..4 {
            let lock = Arc::clone(&lock);
            workers.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    *lock.lock() += 1;
                }
            }));
        }

        for worker in workers {
            worker.join().expect("worker should not panic");
        }

        assert_eq!(*lock.lock(), 4_000);
    }

    #[test]
    fn metrics_record_failed_try_lock_pressure() {
        let lock = SpinLock::new(());
        let _guard = lock.lock();

        assert!(lock.try_lock().is_none());
        assert!(lock.try_lock().is_none());

        let metrics = lock.contention_metrics();
        assert_eq!(metrics.try_lock_attempts(), 3);
        assert_eq!(metrics.failed_try_lock_attempts(), 2);
    }

    #[test]
    fn metrics_record_contention_under_concurrent_pressure() {
        let lock = Arc::new(SpinLock::new(()));
        let guard = lock.lock();

        let worker_lock = Arc::clone(&lock);
        let worker = thread::spawn(move || {
            let mut failed_attempts = 0;
            while worker_lock.try_lock().is_none() {
                failed_attempts += 1;
                if failed_attempts >= 8 {
                    break;
                }
                thread::yield_now();
            }
            failed_attempts
        });

        let failed_attempts = worker.join().expect("worker should not panic");
        assert!(failed_attempts >= 8);

        let metrics = lock.contention_metrics();
        assert!(metrics.observed_contention());

        drop(guard);
        assert!(lock.try_lock().is_some());
    }

    #[test]
    fn initialization_state_allows_one_initializer() {
        let state = InitializationState::new();

        assert!(state.try_begin_initialization());
        assert!(!state.try_begin_initialization());
        assert!(!state.is_initialized());

        state.mark_initialized();

        assert!(state.is_initialized());
        assert!(!state.try_begin_initialization());
    }
}
