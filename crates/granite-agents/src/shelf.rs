//! A folder of installed runners, one folder each with its `installation.json`: what a host reads to offer the
//! agents they carry, without running one. Each call checks the executable before it runs it.
//!
//! PIN: A CHANGED EXECUTABLE IS NOT RUN. An executable whose digest is not the one recorded at install is
//! refused with a sentence; installing it again is the way back. The digest is taken only when the size or
//! modification time moved.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::process::AgentRemoteError;
use crate::{AgentInstallation, RemoteAgent, RosterEntry};

#[derive(Clone, Debug)]
pub struct AgentShelf {
    pub folder: PathBuf,
}

/// A file's size and modification time, seconds since 1970.
pub(crate) fn stamp_of(path: &Path) -> Option<(u64, f64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some((metadata.len(), modified))
}

impl AgentShelf {
    pub fn new(folder: impl Into<PathBuf>) -> AgentShelf {
        AgentShelf {
            folder: folder.into(),
        }
    }

    /// Every installation on the shelf, by name; one that does not read is left out (`problems` says why).
    pub fn installations(&self) -> Vec<AgentInstallation> {
        self.read().0
    }

    /// Sentences for each folder whose installation does not read.
    pub fn problems(&self) -> Vec<String> {
        self.read().1
    }

    /// The installation called `name`, or carrying the agent `name`, matched in any case.
    pub fn installation(&self, name: &str) -> Option<AgentInstallation> {
        let lower = name.to_lowercase();
        let all = self.installations();
        all.iter()
            .find(|installation| installation.name.to_lowercase() == lower)
            .or_else(|| {
                all.iter().find(|installation| {
                    installation
                        .words()
                        .iter()
                        .any(|word| word.to_lowercase() == lower)
                })
            })
            .cloned()
    }

    pub fn folder_of(&self, installation: &AgentInstallation) -> PathBuf {
        self.folder.join(&installation.name)
    }

    pub fn executable_of(&self, installation: &AgentInstallation) -> PathBuf {
        self.folder_of(installation).join(&installation.executable)
    }

    /// Every agent on the shelf, as a roster keeps them: each call one process of its runner, refused when the
    /// executable changed.
    pub fn entries(&self, environment: BTreeMap<String, String>) -> Vec<RosterEntry> {
        self.installations()
            .into_iter()
            .flat_map(|installation| {
                let shelf = self.clone();
                let kept = installation.clone();
                RemoteAgent::entries(
                    &installation.entries,
                    &self.executable_of(&installation),
                    environment.clone(),
                    Arc::new(move || shelf.verify(&kept)),
                )
            })
            .collect()
    }

    /// What changes when an installation is added, removed or rewritten: a host reads the shelf again when it does.
    pub fn stamp(&self) -> String {
        let mut names: Vec<String> = std::fs::read_dir(&self.folder)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| {
                let (_, modified) =
                    stamp_of(&self.folder.join(&name).join(AgentInstallation::FILE))?;
                Some(format!("{name}@{modified}"))
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    /// `Changed` when the executable is not the one installed.
    pub fn verify(&self, installation: &AgentInstallation) -> Result<(), AgentRemoteError> {
        let path = self.executable_of(installation);
        let word = installation
            .words()
            .into_iter()
            .next()
            .unwrap_or_else(|| installation.name.clone());
        let changed = || AgentRemoteError::Changed {
            word: word.clone(),
            path: path.display().to_string(),
        };
        let Some((size, modified)) = stamp_of(&path) else {
            return Err(changed());
        };
        if size == installation.size
            && installation
                .modified
                .is_some_and(|recorded| (modified - recorded).abs() < 0.001)
        {
            return Ok(());
        }
        match Self::sha256(&path) {
            Ok(digest) if digest == installation.sha256 => Ok(()),
            _ => Err(changed()),
        }
    }

    /// A file's SHA-256, lowercase hex.
    pub fn sha256(path: &Path) -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut hash = Sha256::new();
        let mut chunk = vec![0u8; 1 << 20];
        loop {
            let count = file.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            hash.update(&chunk[..count]);
        }
        Ok(hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    fn read(&self) -> (Vec<AgentInstallation>, Vec<String>) {
        let mut names: Vec<String> = std::fs::read_dir(&self.folder)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.retain(|name| !name.starts_with('.'));
        names.sort();
        let (mut installations, mut problems) = (Vec::new(), Vec::new());
        for name in names {
            let manifest = self.folder.join(&name).join(AgentInstallation::FILE);
            let Ok(bytes) = std::fs::read(&manifest) else {
                continue;
            };
            match serde_json::from_slice::<AgentInstallation>(&bytes) {
                Ok(installation) if installation.format.starts_with(AgentInstallation::FORMAT) => {
                    installations.push(installation)
                }
                Ok(installation) => problems.push(format!(
                    "{name}: {} is not an installation this host reads",
                    installation.format
                )),
                Err(error) => problems.push(format!(
                    "{name}: its {} does not read: {error}",
                    AgentInstallation::FILE
                )),
            }
        }
        (installations, problems)
    }
}
