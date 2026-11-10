//! State behind a lock: Swift's `GraniteLock<State>`.
//!
//! PIN: NOTHING IS DROPPED UNDER A LOCK. A drop runs whatever the dropped value owned alone (a
//! handler's captures, a run's compute), and std's `Mutex` is not reentrant: code that reached this
//! lock again from a destructor would deadlock or panic. [`Locked::with`] hands the body's result back
//! after the guard is gone, so a body that returns what it took out lets it go after the unlock.
//! In `x.lock().unwrap().remove(k);` the removed value is a temporary dropped before the guard; through
//! `with(|x| x.remove(k))` it is dropped after.

use std::sync::Mutex;

/// A value and the lock that guards it. The body of [`Locked::with`] never calls out and never drops
/// what it removes: it returns it.
#[derive(Default)]
pub(crate) struct Locked<T>(Mutex<T>);

impl<T> Locked<T> {
    pub(crate) const fn new(value: T) -> Self {
        Locked(Mutex::new(value))
    }

    /// Runs `body` with the lock held; what it returns is handed back once the lock is released.
    pub(crate) fn with<R>(&self, body: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.0.lock().unwrap();
        let result = body(&mut guard);
        drop(guard);
        result
    }
}
