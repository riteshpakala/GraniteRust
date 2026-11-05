use std::fmt;

use crate::AgentRefusal;

/// What a call to an agent in another process can say besides its output.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AgentRemoteError {
    /// The agent ran and failed, and said why: from a runner that keeps no answer standard.
    Failed { word: String, message: String },
    /// The agent refused the call, with its kind and reasons, in the sentence it said.
    Refused {
        word: String,
        refusal: AgentRefusal,
        message: String,
    },
    /// The agent failed by its own fault, not the call's.
    Fault { word: String, message: String },
    /// The process ended without answering: crashed, killed, or never spoke the wire.
    Ended {
        word: String,
        status: String,
        log: String,
    },
    /// The process said something the wire does not allow where it said it.
    Unreadable { word: String, reason: String },
    /// The executable is not the one that was installed.
    Changed { word: String, path: String },
    /// The executable could not be started.
    Spawn { path: String, reason: String },
}

impl fmt::Display for AgentRemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentRemoteError::Failed { word, message } => write!(f, "{word} failed: {message}"),
            AgentRemoteError::Refused {
                word,
                refusal,
                message,
            } => write!(f, "{word} refused ({}): {message}", refusal.kind.as_str()),
            AgentRemoteError::Fault { word, message } => {
                write!(f, "{word} failed by its own fault: {message}")
            }
            AgentRemoteError::Ended { word, status, log } => {
                write!(f, "{word}'s process ended ({status}) without answering")?;
                if !log.is_empty() {
                    write!(f, ": {log}")?;
                }
                Ok(())
            }
            AgentRemoteError::Unreadable { word, reason } => {
                write!(f, "{word}'s process broke the wire: {reason}")
            }
            AgentRemoteError::Changed { word, path } => write!(
                f,
                "{word}'s executable changed since it was installed ({path}); install it again"
            ),
            AgentRemoteError::Spawn { path, reason } => {
                write!(f, "could not start {path}: {reason}")
            }
        }
    }
}

impl std::error::Error for AgentRemoteError {}
