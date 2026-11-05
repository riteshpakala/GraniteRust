use std::fmt;
use std::path::{Component, PathBuf};

use serde::{Deserialize, Serialize};

/// Where an agent package's code is: a git repository, at a revision or its default branch's head, or a folder.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentSource {
    Path {
        path: String,
    },
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<String>,
    },
}

impl AgentSource {
    /// `https://github.com/rao/ComicAgent.git`, `…/ComicAgent.git@f9bdda7`, `git@github.com:rao/ComicAgent.git`, or
    /// a folder: `/abs/path`, `~/path`, `./path`, `../path`, resolved against `base`.
    pub fn parse(said: &str, base: &std::path::Path) -> AgentSource {
        let said = said.trim();
        if said.starts_with('/') || said.starts_with('~') || said.starts_with('.') {
            let expanded = match said.strip_prefix('~') {
                Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default())
                    .join(rest.trim_start_matches('/')),
                None => PathBuf::from(said),
            };
            let joined = if expanded.is_absolute() {
                expanded
            } else {
                base.join(expanded)
            };
            let mut clean = PathBuf::from("/");
            for component in joined.components() {
                match component {
                    Component::ParentDir => {
                        clean.pop();
                    }
                    Component::Normal(part) => clean.push(part),
                    _ => {}
                }
            }
            return AgentSource::Path {
                path: clean.to_string_lossy().into_owned(),
            };
        }
        // A revision follows the last `@` after the last `/`: `git@host:x/y.git` has none.
        match (said.rfind('@'), said.rfind('/')) {
            (Some(at), Some(slash)) if at > slash => AgentSource::Git {
                url: said[..at].to_owned(),
                revision: Some(said[at + 1..].to_owned()),
            },
            _ => AgentSource::Git {
                url: said.to_owned(),
                revision: None,
            },
        }
    }

    /// The same source at its newest: a repository's head, a folder as it is.
    pub fn newest(&self) -> AgentSource {
        match self {
            AgentSource::Git { url, .. } => AgentSource::Git {
                url: url.clone(),
                revision: None,
            },
            path => path.clone(),
        }
    }
}

impl fmt::Display for AgentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentSource::Git {
                url,
                revision: Some(revision),
            } => write!(f, "{url}@{revision}"),
            AgentSource::Git {
                url,
                revision: None,
            } => f.write_str(url),
            AgentSource::Path { path } => f.write_str(path),
        }
    }
}
