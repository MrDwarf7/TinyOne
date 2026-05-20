use std::sync::{Arc, Condvar, Mutex};

use crate::{Result, TinyOneError, Value};

pub(crate) struct TinyMutex {
    state: Mutex<bool>, // true = locked
    cond:  Condvar,
}

impl TinyMutex {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(false),
            cond:  Condvar::new(),
        })
    }

    /// Block until the mutex is unlocked, then acquire it.
    pub(crate) fn lock(&self) -> Result<()> {
        let mut locked = self.state.lock()
            .map_err(|_| TinyOneError::runtime("mutex_lock: mutex poisoned"))?;
        locked = self.cond
            .wait_while(locked, |l| *l)
            .map_err(|_| TinyOneError::runtime("mutex_lock: condvar wait failed"))?;
        *locked = true;
        Ok(())
    }

    /// Release the mutex. Returns a runtime error if not currently locked.
    pub(crate) fn unlock(&self) -> Result<()> {
        let mut locked = self.state.lock()
            .map_err(|_| TinyOneError::runtime("mutex_unlock: mutex poisoned"))?;
        if !*locked {
            return Err(TinyOneError::runtime("mutex_unlock: mutex is not locked"));
        }
        *locked = false;
        self.cond.notify_one();
        Ok(())
    }

    pub(crate) fn is_locked(&self) -> bool {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(crate) struct TinyThreadHandle {
    pub(crate) inner: Mutex<Option<std::thread::JoinHandle<(Result<Value>, Vec<u8>)>>>,
}

impl TinyThreadHandle {
    pub(crate) fn new(
        handle: std::thread::JoinHandle<(Result<Value>, Vec<u8>)>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Some(handle)),
        })
    }

    /// Block until thread finishes. Returns (return_value, stdout_bytes).
    /// Runtime error if called more than once.
    pub(crate) fn join(&self) -> Result<(Value, Vec<u8>)> {
        let handle = self.inner
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
