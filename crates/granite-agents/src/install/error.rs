use std::fmt;

/// What installing an agent package can say instead of an installation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AgentInstallError {
    Toolchain(String),
    Fetch {
        source: String,
        reason: String,
    },
    NotAPackage {
        source: String,
        reason: String,
    },
    NotGranite {
        name: String,
    },
    Build {
        name: String,
        log: String,
        path: String,
    },
    NoAgent {
        name: String,
    },
    Clash {
        word: String,
        installed: String,
    },
    NotInstalled(String),
}

impl fmt::Display for AgentInstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentInstallError::Toolchain(reason) => write!(
                f,
                "a Rust toolchain (cargo) is needed to build an agent: {reason}"
            ),
            AgentInstallError::Fetch { source, reason } => {
                write!(f, "could not fetch {source}: {reason}")
            }
            AgentInstallError::NotAPackage { source, reason } => {
                write!(f, "{source} is not a Cargo package: {reason}")
            }
            AgentInstallError::NotGranite { name } => write!(
                f,
                "{name} does not depend on granite-agents, so it carries no Granite agent"
            ),
            AgentInstallError::Build { name, log, path } => write!(
                f,
                "{name} did not build (the whole log is at {path}):\n{log}"
            ),
            AgentInstallError::NoAgent { name } => {
                write!(f, "{name} holds no agent: its runner describes none")
            }
            AgentInstallError::Clash { word, installed } => write!(
                f,
                "an agent called `{word}` is already installed, from {installed}"
            ),
            AgentInstallError::NotInstalled(name) => {
                write!(f, "no agent installed here is called {name}")
            }
        }
    }
}

impl std::error::Error for AgentInstallError {}
