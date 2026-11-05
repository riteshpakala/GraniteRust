//! An agent package made a runner on this machine: fetched, read, built in release, asked what it carries, and
//! installed on a shelf with its record. The Cargo analogue of Swift Granite's `AgentInstaller`.
//!
//! ```text
//! fetch → cargo metadata → cargo build --release --bin <runner> → <runner> --describe → install
//! ```
//!
//! PIN: THE PACKAGE SHIPS ITS RUNNER. A Swift package's runner is generated; a Rust package names its own bin in
//! `[package.metadata.granite] agent = "<bin>"` (or has one bin, or a bin called `agent`), whose `main` is
//! `AgentServer::main(roster![…])`. Installed, it is always called `agent`, as a Swift runner is.
//! PIN: ONE WORD, ONE INSTALLATION. A package that carries a word another installation carries is refused;
//! installing the same package again replaces it whole.
//! PIN: THE STAGES ARE SWIFT'S WORDS (`fetching`, `reading the package`, `building`, `asking … what it carries`,
//! `installing`, `installed`), so a host that shows Swift installs reads these the same way.

mod command;
mod error;
mod installation;
mod manifest;
mod source;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use error::AgentInstallError;
pub use installation::{AgentInstallation, now_iso8601};
pub use source::AgentSource;

use command::run;
use manifest::CargoManifest;

use crate::shelf::stamp_of;
use crate::{AgentProcess, AgentShelf, json};

/// A step of an install, with the line the tool just said.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Progress {
    pub stage: String,
    pub line: Option<String>,
}

impl Progress {
    fn stage(stage: impl Into<String>) -> Progress {
        Progress {
            stage: stage.into(),
            line: None,
        }
    }
}

pub struct AgentInstaller {
    pub shelf: AgentShelf,
}

/// Removes the work folder however the install ends.
struct Work(PathBuf);

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .join("\n")
        .trim()
        .to_owned()
}

impl AgentInstaller {
    pub fn new(folder: impl Into<PathBuf>) -> AgentInstaller {
        AgentInstaller {
            shelf: AgentShelf::new(folder),
        }
    }

    /// Installs the package at `source`, replacing an installation of the same name.
    pub async fn install(
        &self,
        source: AgentSource,
        progress: impl Fn(Progress) + Send + Sync + 'static,
    ) -> Result<AgentInstallation, AgentInstallError> {
        let progress = Arc::new(progress);
        let builds = self.shelf.folder.join(".builds");
        let work = builds.join(format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&work).map_err(|error| AgentInstallError::Fetch {
            source: source.to_string(),
            reason: error.to_string(),
        })?;
        let _work = Work(work.clone());

        // 1. Fetch.
        let (package, revision): (PathBuf, Option<String>) = match &source {
            AgentSource::Git {
                url,
                revision: wanted,
            } => {
                progress(Progress::stage(format!("fetching {url}")));
                let name = url
                    .trim_end_matches('/')
                    .rsplit(['/', ':'])
                    .next()
                    .unwrap_or("package")
                    .trim_end_matches(".git")
                    .to_owned();
                let package = work.join(if name.is_empty() { "package" } else { &name });
                let clone = run(
                    &["git", "clone", "--quiet", url, &package.to_string_lossy()],
                    &work,
                    None,
                )
                .await?;
                if !clone.ok() {
                    return Err(AgentInstallError::Fetch {
                        source: url.clone(),
                        reason: tail(&clone.output, 4),
                    });
                }
                if let Some(wanted) = wanted {
                    let checkout =
                        run(&["git", "checkout", "--quiet", wanted], &package, None).await?;
                    if !checkout.ok() {
                        return Err(AgentInstallError::Fetch {
                            source: format!("{url}@{wanted}"),
                            reason: tail(&checkout.output, 4),
                        });
                    }
                }
                let revision = head(&package).await;
                (package, revision)
            }
            AgentSource::Path { path } => {
                let package = PathBuf::from(path);
                if !package.join("Cargo.toml").exists() {
                    return Err(AgentInstallError::NotAPackage {
                        source: path.clone(),
                        reason: "it has no Cargo.toml".into(),
                    });
                }
                (package.clone(), head(&package).await)
            }
        };

        // 2. Read the manifest.
        progress(Progress::stage("reading the package"));
        let manifest_path = package.join("Cargo.toml");
        let metadata = run(
            &[
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--color",
                "never",
                "--manifest-path",
                &manifest_path.to_string_lossy(),
            ],
            &work,
            None,
        )
        .await?;
        if metadata.status == 127 {
            return Err(AgentInstallError::Toolchain("`cargo` was not found".into()));
        }
        if !metadata.ok() {
            return Err(AgentInstallError::NotAPackage {
                source: source.to_string(),
                reason: tail(&metadata.output, 6),
            });
        }
        let read = CargoManifest::read(&metadata.stdout, &package, &source.to_string())?;

        // 3. Build: the package's own runner, in release, in a target shared by installs so an update is quick.
        progress(Progress::stage(format!("building {}", read.name)));
        let target = builds.join("cargo-target");
        let building = progress.clone();
        let stage = format!("building {}", read.name);
        let build = run(
            &[
                "cargo",
                "build",
                "--release",
                "--color",
                "never",
                "--manifest-path",
                &read.manifest_path.to_string_lossy(),
                "-p",
                &read.package,
                "--bin",
                &read.bin,
                "--target-dir",
                &target.to_string_lossy(),
            ],
            &package,
            Some(Box::new(move |line: &str| {
                building(Progress {
                    stage: stage.clone(),
                    line: Some(line.to_owned()),
                })
            })),
        )
        .await?;
        if !build.ok() {
            let log = builds.join(format!("{}.log", read.name));
            let _ = std::fs::write(&log, &build.output);
            return Err(AgentInstallError::Build {
                name: read.name.clone(),
                log: tail(&build.output, 12),
                path: log.display().to_string(),
            });
        }
        let built = target.join("release").join(&read.bin);

        // 4. Describe and check.
        progress(Progress::stage(format!(
            "asking {} what it carries",
            read.name
        )));
        let entries = AgentProcess::new(&built)
            .describe()
            .await
            .map_err(|error| AgentInstallError::NoAgent {
                name: format!("{} ({error})", read.name),
            })?;
        if entries.is_empty() {
            return Err(AgentInstallError::NoAgent {
                name: read.name.clone(),
            });
        }
        for other in self
            .shelf
            .installations()
            .into_iter()
            .filter(|other| other.name != read.name)
        {
            if let Some(word) = entries
                .iter()
                .map(|entry| entry.word.clone())
                .find(|word| other.words().contains(word))
            {
                return Err(AgentInstallError::Clash {
                    word,
                    installed: other.source.to_string(),
                });
            }
        }

        // 5. Install, whole or not at all.
        progress(Progress::stage(format!("installing {}", read.name)));
        let failed = |error: std::io::Error| AgentInstallError::Fetch {
            source: source.to_string(),
            reason: format!("could not install: {error}"),
        };
        let staged = self.shelf.folder.join(format!(".{}.new", read.name));
        let _ = std::fs::remove_dir_all(&staged);
        std::fs::create_dir_all(&staged).map_err(failed)?;
        let executable = staged.join("agent");
        std::fs::copy(&built, &executable).map_err(failed)?;
        let (size, modified) = stamp_of(&executable)
            .ok_or_else(|| failed(std::io::Error::other("the copied runner is not there")))?;
        let installation = AgentInstallation {
            format: AgentInstallation::FORMAT.into(),
            name: read.name.clone(),
            source: source.clone(),
            revision,
            granite: read.granite.clone(),
            executable: "agent".into(),
            sha256: AgentShelf::sha256(&executable).map_err(failed)?,
            size,
            modified: Some(modified),
            built_at: now_iso8601(),
            entries: entries.clone(),
        };
        std::fs::write(
            staged.join(AgentInstallation::FILE),
            json::pretty(&installation).map_err(|error| failed(std::io::Error::other(error)))?,
        )
        .map_err(failed)?;
        let destination = self.shelf.folder.join(&read.name);
        let _ = std::fs::remove_dir_all(&destination);
        std::fs::rename(&staged, &destination).map_err(failed)?;
        let words: Vec<String> = entries.iter().map(|entry| entry.word.clone()).collect();
        progress(Progress::stage(format!(
            "installed {}: {}",
            read.name,
            words.join(", ")
        )));
        Ok(installation)
    }

    /// Installs the installation called `name` again, from its source at its newest.
    pub async fn update(
        &self,
        name: &str,
        progress: impl Fn(Progress) + Send + Sync + 'static,
    ) -> Result<AgentInstallation, AgentInstallError> {
        let installed = self
            .shelf
            .installation(name)
            .ok_or_else(|| AgentInstallError::NotInstalled(name.into()))?;
        self.install(installed.source.newest(), progress).await
    }

    /// Removes the installation called `name`, or carrying the agent `name`.
    pub fn remove(&self, name: &str) -> Result<AgentInstallation, AgentInstallError> {
        let installed = self
            .shelf
            .installation(name)
            .ok_or_else(|| AgentInstallError::NotInstalled(name.into()))?;
        std::fs::remove_dir_all(self.shelf.folder_of(&installed)).map_err(|error| {
            AgentInstallError::Fetch {
                source: installed.name.clone(),
                reason: error.to_string(),
            }
        })?;
        Ok(installed)
    }
}

async fn head(package: &Path) -> Option<String> {
    let said = run(&["git", "rev-parse", "HEAD"], package, None)
        .await
        .ok()?;
    said.ok()
        .then(|| said.stdout.trim().to_owned())
        .filter(|revision| !revision.is_empty())
}
