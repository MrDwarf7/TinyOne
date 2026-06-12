use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};

use crate::{Result, TinyOneError, Value};

#[derive(Debug)]
pub(crate) struct TinyMutex {
    // None = unlocked; Some(tid) = locked by thread tid
    state: Mutex<Option<ThreadId>>,
    cond:  Condvar,
}

impl TinyMutex {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(None),
            cond:  Condvar::new(),
        })
    }

    /// Block until the mutex is unlocked, then acquire it.
    /// Returns a runtime error if the calling thread already holds this mutex (deadlock).
    pub(crate) fn lock(&self) -> Result<()> {
        let current = thread::current().id();
        let mut state = self
            .state
            .lock()
            .map_err(|_| TinyOneError::runtime("mutex_lock: mutex poisoned"))?;
        if *state == Some(current) {
            return Err(TinyOneError::runtime("mutex_lock: deadlock — already locked by this thread"));
        }
        state = self
            .cond
            .wait_while(state, |s| s.is_some())
            .map_err(|_| TinyOneError::runtime("mutex_lock: condvar wait failed"))?;
        *state = Some(current);
        Ok(())
    }

    /// Release the mutex. Returns a runtime error if not currently locked.
    pub(crate) fn unlock(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| TinyOneError::runtime("mutex_unlock: mutex poisoned"))?;
        if state.is_none() {
            return Err(TinyOneError::runtime("mutex_unlock: mutex is not locked"));
        }
        *state = None;
        self.cond.notify_one();
        Ok(())
    }

    pub(crate) fn is_locked(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }
}

pub(crate) struct TinyThreadHandle {
    pub(crate) inner: Mutex<Option<std::thread::JoinHandle<(Result<Value>, Vec<u8>)>>>,
}

impl fmt::Debug for TinyThreadHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TinyThreadHandle")
    }
}

impl TinyThreadHandle {
    pub(crate) fn new(handle: std::thread::JoinHandle<(Result<Value>, Vec<u8>)>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Some(handle)),
        })
    }

    /// Block until thread finishes. Returns (return_value, stdout_bytes).
    /// Runtime error if called more than once.
    pub(crate) fn join(&self) -> Result<(Value, Vec<u8>)> {
        let handle = self
            .inner
            .lock()
            .map_err(|_| TinyOneError::runtime("thread_join: handle mutex poisoned"))?
            .take()
            .ok_or_else(|| TinyOneError::runtime("thread_join: already joined"))?;
        match handle.join() {
            Ok((Ok(value), stdout)) => Ok((value, stdout)),
            Ok((Err(e), _)) => Err(e),
            Err(_) => Err(TinyOneError::runtime("thread_join: thread panicked")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tinymutex_lock_unlock_roundtrip() {
        let m = TinyMutex::new();
        assert!(!m.is_locked());
        m.lock().unwrap();
        assert!(m.is_locked());
        m.unlock().unwrap();
        assert!(!m.is_locked());
    }

    #[test]
    fn tinymutex_double_unlock_is_error() {
        let m = TinyMutex::new();
        m.lock().unwrap();
        m.unlock().unwrap();
        assert!(m.unlock().is_err());
    }

    #[test]
    fn tinymutex_cross_thread_blocking() {
        use std::sync::Arc;
        let m = TinyMutex::new();
        m.lock().unwrap();
        let m2 = Arc::clone(&m);
        let t = std::thread::spawn(move || {
            m2.lock().unwrap();
            m2.unlock().unwrap();
        });
        m.unlock().unwrap();
        t.join().unwrap();
    }
}
