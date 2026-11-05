//! Where a persisted center keeps its state between launches.

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Reads a center's state when it is made, and writes it on each commit that changes it.
pub trait Persistence<S>: Send + Sync {
    fn load(&self) -> Option<S>;
    fn save(&self, state: &S);
}

/// A state kept as JSON in one file, written whole (to a sibling, then renamed) so a reader never sees half.
pub struct FilePersistence {
    pub path: PathBuf,
}

impl FilePersistence {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FilePersistence { path: path.into() }
    }
}

impl<S: Serialize + DeserializeOwned> Persistence<S> for FilePersistence {
    fn load(&self) -> Option<S> {
        let bytes = std::fs::read(&self.path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save(&self, state: &S) {
        let Ok(bytes) = serde_json::to_vec_pretty(state) else {
            return;
        };
        if let Some(folder) = self.path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let staged = self.path.with_extension("saving");
        if std::fs::write(&staged, bytes).is_ok() {
            let _ = std::fs::rename(&staged, &self.path);
        }
    }
}
