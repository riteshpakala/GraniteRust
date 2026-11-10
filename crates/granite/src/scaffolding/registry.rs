//! Cancellable work by id, and the residents a state cannot hold, by key.
//!
//! PIN: NOTHING IS DROPPED OR MADE UNDER THE LOCK. A resident replaced or removed, a task handle let go,
//! and the value `make` builds are all handled after the unlock: a value's drop, or its making, may reach
//! the same registry.

use std::collections::{HashMap, HashSet};

use tokio::task::JoinHandle;

use crate::locked::Locked;

/// Long work by id: Swift's `RunRegistry`.
///
/// PIN: a cancel that beats its register leaves a tombstone: `register` then aborts the task and says
/// false. `finish` clears both.
#[derive(Default)]
pub struct RunRegistry {
    inner: Locked<Runs>,
}

/// The tasks by id, and the ids whose cancel came first.
#[derive(Default)]
struct Runs {
    tasks: HashMap<String, JoinHandle<()>>,
    cancelled: HashSet<String>,
}

impl RunRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps `task` under `id`; false (and the task aborted) when a cancel came first.
    pub fn register(&self, id: &str, task: JoinHandle<()>) -> bool {
        let refused = self.inner.with(|inner| {
            if inner.cancelled.contains(id) {
                return Err(task);
            }
            Ok(inner.tasks.insert(id.to_owned(), task))
        });
        match refused {
            Err(task) => {
                task.abort();
                false
            }
            Ok(replaced) => {
                drop(replaced);
                true
            }
        }
    }

    /// Aborts the task under `id`, or leaves a tombstone for one not registered yet.
    pub fn cancel(&self, id: &str) {
        let abort = self.inner.with(|inner| {
            inner.cancelled.insert(id.to_owned());
            inner.tasks.get(id).map(JoinHandle::abort_handle)
        });
        if let Some(abort) = abort {
            abort.abort();
        }
    }

    pub fn is_cancelled(&self, id: &str) -> bool {
        self.inner.with(|inner| inner.cancelled.contains(id))
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.inner.with(|inner| inner.tasks.contains_key(id))
    }

    /// The work ended: its task and its tombstone go.
    pub fn finish(&self, id: &str) {
        let task = self.inner.with(|inner| {
            inner.cancelled.remove(id);
            inner.tasks.remove(id)
        });
        drop(task);
    }
}

/// Residents by key: Swift's `TransientRegistry`. A lock, not an actor: a reducer reads it in place.
pub struct TransientRegistry<V> {
    items: Locked<HashMap<String, V>>,
}

impl<V> Default for TransientRegistry<V> {
    fn default() -> Self {
        TransientRegistry {
            items: Locked::new(HashMap::new()),
        }
    }
}

impl<V: Clone> TransientRegistry<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, key: &str, value: V) {
        let replaced = self.items.with(|items| items.insert(key.to_owned(), value));
        drop(replaced);
    }

    pub fn get(&self, key: &str) -> Option<V> {
        self.items.with(|items| items.get(key).cloned())
    }

    /// The value under `key`, made by `make` when there is none: one per key. `make` runs outside the
    /// lock, so two callers racing on a new key may both make; the first stored is kept and returned to
    /// both, and the other is dropped.
    pub fn value(&self, key: &str, make: impl FnOnce() -> V) -> V {
        if let Some(found) = self.get(key) {
            return found;
        }
        let made = make();
        let (kept, lost) = self.items.with(|items| match items.get(key) {
            Some(found) => (found.clone(), Some(made)),
            None => {
                items.insert(key.to_owned(), made.clone());
                (made, None)
            }
        });
        drop(lost);
        kept
    }

    pub fn remove(&self, key: &str) -> Option<V> {
        self.items.with(|items| items.remove(key))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.items.with(|items| items.contains_key(key))
    }

    pub fn keys(&self, prefix: &str) -> Vec<String> {
        let mut keys: Vec<String> = self.items.with(|items| {
            items
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect()
        });
        keys.sort();
        keys
    }

    pub fn values(&self) -> Vec<V> {
        self.items.with(|items| items.values().cloned().collect())
    }

    /// Removes and returns every value whose key starts with `prefix`.
    pub fn remove_all_prefixed(&self, prefix: &str) -> Vec<V> {
        self.items.with(|items| {
            let gone: Vec<String> = items
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect();
            gone.into_iter()
                .filter_map(|key| items.remove(&key))
                .collect()
        })
    }

    pub fn remove_all(&self) {
        let removed = self.items.with(std::mem::take);
        drop(removed);
    }
}
