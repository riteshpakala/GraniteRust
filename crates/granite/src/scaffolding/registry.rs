//! Cancellable work by id, and the residents a state cannot hold, by key.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use tokio::task::JoinHandle;

/// Long work by id: Swift's `RunRegistry`.
///
/// PIN: a cancel that beats its register leaves a tombstone: `register` then aborts the task and says
/// false. `finish` clears both.
#[derive(Default)]
pub struct RunRegistry {
    inner: Mutex<Runs>,
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
        let mut inner = self.inner.lock().unwrap();
        if inner.cancelled.contains(id) {
            task.abort();
            return false;
        }
        inner.tasks.insert(id.to_owned(), task);
        true
    }

    /// Aborts the task under `id`, or leaves a tombstone for one not registered yet.
    pub fn cancel(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(task) = inner.tasks.get(id) {
            task.abort();
        }
        inner.cancelled.insert(id.to_owned());
    }

    pub fn is_cancelled(&self, id: &str) -> bool {
        self.inner.lock().unwrap().cancelled.contains(id)
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.inner.lock().unwrap().tasks.contains_key(id)
    }

    /// The work ended: its task and its tombstone go.
    pub fn finish(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.tasks.remove(id);
        inner.cancelled.remove(id);
    }
}

/// Residents by key: Swift's `TransientRegistry`. A lock, not an actor: a reducer reads it in place.
pub struct TransientRegistry<V> {
    items: Mutex<HashMap<String, V>>,
}

impl<V> Default for TransientRegistry<V> {
    fn default() -> Self {
        TransientRegistry {
            items: Mutex::new(HashMap::new()),
        }
    }
}

impl<V: Clone> TransientRegistry<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, key: &str, value: V) {
        self.items.lock().unwrap().insert(key.to_owned(), value);
    }

    pub fn get(&self, key: &str) -> Option<V> {
        self.items.lock().unwrap().get(key).cloned()
    }

    /// The value under `key`, made by `make` under the lock when there is none: one per key.
    pub fn value(&self, key: &str, make: impl FnOnce() -> V) -> V {
        self.items
            .lock()
            .unwrap()
            .entry(key.to_owned())
            .or_insert_with(make)
            .clone()
    }

    pub fn remove(&self, key: &str) -> Option<V> {
        self.items.lock().unwrap().remove(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.items.lock().unwrap().contains_key(key)
    }

    pub fn keys(&self, prefix: &str) -> Vec<String> {
        let mut keys: Vec<String> = self
            .items
            .lock()
            .unwrap()
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    pub fn values(&self) -> Vec<V> {
        self.items.lock().unwrap().values().cloned().collect()
    }

    /// Removes and returns every value whose key starts with `prefix`.
    pub fn remove_all_prefixed(&self, prefix: &str) -> Vec<V> {
        let mut items = self.items.lock().unwrap();
        let gone: Vec<String> = items
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect();
        gone.into_iter()
            .filter_map(|key| items.remove(&key))
            .collect()
    }

    pub fn remove_all(&self) {
        self.items.lock().unwrap().clear();
    }
}
