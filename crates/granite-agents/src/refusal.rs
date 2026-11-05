//! Why a call was not answered, as a kind a host can count and reasons each saying its fix.
//!
//! PIN: A REFUSAL IS NOT A FAULT. An error that is neither an [`AgentRefusal`] nor said as one
//! ([`AgentRefusing`], [`SaidRefusal`]) is the agent's fault, said as it is; only a refusal is the call's
//! to correct.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::package::AgentPackageError;
use crate::process::AgentRemoteError;
use crate::{AgentHostError, BoxError};

/// Whose a refusal is: what a host tells its model.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefusalKind {
    /// The call's JSON does not read as the input.
    Input,
    /// The call cannot be honoured as written: the fix is the caller's.
    Call,
    /// A build the call needs failed.
    Build,
    /// A device the call needs failed: a GPU, a simulator.
    Device,
    /// The host's model gave nothing usable.
    Model,
    /// Code the package wrote did not parse, so nothing was written.
    Parse,
    /// A service it reads could not be reached: try later.
    Reach,
    /// Its answer broke `granite-answer/1`: the package's own bug.
    Answer,
    /// The host could not serve it; also what a kind this reader does not know reads as.
    #[serde(other)]
    Host,
}

impl RefusalKind {
    pub const ALL: [RefusalKind; 9] = [
        RefusalKind::Input,
        RefusalKind::Call,
        RefusalKind::Build,
        RefusalKind::Device,
        RefusalKind::Model,
        RefusalKind::Parse,
        RefusalKind::Reach,
        RefusalKind::Answer,
        RefusalKind::Host,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RefusalKind::Input => "input",
            RefusalKind::Call => "call",
            RefusalKind::Build => "build",
            RefusalKind::Device => "device",
            RefusalKind::Model => "model",
            RefusalKind::Parse => "parse",
            RefusalKind::Reach => "reach",
            RefusalKind::Answer => "answer",
            RefusalKind::Host => "host",
        }
    }
}

/// A refusal: its kind, and one sentence a reason, its fix in it.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct AgentRefusal {
    pub kind: RefusalKind,
    pub reasons: Vec<String>,
}

impl AgentRefusal {
    pub fn new(kind: RefusalKind, reasons: impl IntoIterator<Item = impl Into<String>>) -> Self {
        AgentRefusal {
            kind,
            reasons: reasons.into_iter().map(Into::into).collect(),
        }
    }

    pub fn one(kind: RefusalKind, reason: impl Into<String>) -> Self {
        AgentRefusal {
            kind,
            reasons: vec![reason.into()],
        }
    }

    /// The refusal `error` is, when it is one; none: a fault.
    pub fn of(error: &(dyn std::error::Error + 'static)) -> Option<AgentRefusal> {
        if let Some(refusal) = error.downcast_ref::<AgentRefusal>() {
            return Some(refusal.clone());
        }
        if let Some(said) = error.downcast_ref::<SaidRefusal>() {
            return Some(said.refusal.clone());
        }
        if let Some(package) = error.downcast_ref::<AgentPackageError>() {
            return match package {
                AgentPackageError::Input { reason, .. } => {
                    Some(AgentRefusal::one(RefusalKind::Input, reason.clone()))
                }
                AgentPackageError::NoOutput { word } => Some(AgentRefusal::one(
                    RefusalKind::Answer,
                    format!("{word} finished without an answer"),
                )),
                AgentPackageError::Refused { refusal, .. } => Some(refusal.clone()),
                AgentPackageError::Cancelled | AgentPackageError::Fault(_) => None,
            };
        }
        if let Some(AgentRemoteError::Refused { refusal, .. }) =
            error.downcast_ref::<AgentRemoteError>()
        {
            return Some(refusal.clone());
        }
        if let Some(host) = error.downcast_ref::<AgentHostError>() {
            return Some(AgentRefusal::one(RefusalKind::Model, host.to_string()));
        }
        None
    }
}

impl fmt::Display for AgentRefusal {
    /// One reason as it is; several as a count and a list.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.reasons.len() == 1 {
            return f.write_str(&self.reasons[0]);
        }
        write!(f, "{} problems, so nothing was done:", self.reasons.len())?;
        for reason in &self.reasons {
            write!(f, "\n- {reason}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AgentRefusal {}

/// A refusal said in a package's own sentence: what [`AgentRefusing::refused`] makes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SaidRefusal {
    pub refusal: AgentRefusal,
    pub said: String,
}

impl fmt::Display for SaidRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.said)
    }
}

impl std::error::Error for SaidRefusal {}

/// A package's own error type that is a refusal: `impl AgentRefusing for ComicError { … }`, then
/// `Err(error.refused())` from a run.
pub trait AgentRefusing: std::error::Error + Sized {
    fn refusal(&self) -> AgentRefusal;

    /// The error as a run returns it, its kind kept and said in its own sentence.
    fn refused(self) -> BoxError {
        Box::new(SaidRefusal {
            refusal: self.refusal(),
            said: self.to_string(),
        })
    }
}
