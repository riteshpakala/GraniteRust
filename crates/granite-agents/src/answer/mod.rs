//! `granite-answer/1`: one JSON object whose reserved keys a host reads, beside the package's own.
//!
//! | Key | Who writes it | What it holds |
//! | --- | --- | --- |
//! | `format` | the runner | `granite-answer/1` |
//! | `agent` | the runner | `{"word": …}` |
//! | `path` | the package | the first file it wrote, or null |
//! | `paths` | the package | every file it wrote or removed; `[]` when none |
//! | `notes` | the package | what the model should know, one sentence each |
//! | `made` | the package | what it made: `kind` (one of its `makes`), `name`, `path`, `state` |
//! | `expected` | the package | what its call asked for, as it read the call: `kind`, `name` |
//! | `problems` | the package | what it could only do in part: `kind`, `text`; not a refusal |
//! | `payloads` | the package | files among `paths` that keep a standard of their own: `path`, `schema` |

mod check;

pub use check::AnswerCheck;

use serde::{Deserialize, Serialize};

/// An output held to `granite-answer/1`: checked and stamped before it leaves the process. Every Rust
/// package's output is one (Swift's opt-in is this port's rule).
pub trait AgentAnswer: Serialize + Send + 'static {}

/// The standard's ids and reserved keys.
pub struct AnswerStandard;

impl AnswerStandard {
    pub const FORMAT: &'static str = "granite-answer/1";
    pub const RESERVED: [&'static str; 9] = [
        "format", "agent", "path", "paths", "notes", "made", "expected", "problems", "payloads",
    ];
    /// The keys the runner writes, never the package.
    pub const STAMPED: [&'static str; 2] = ["format", "agent"];
}

/// How a made file stands after the call.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MadeState {
    #[default]
    Written,
    Rewritten,
    Unchanged,
    Removed,
}

impl MadeState {
    pub const ALL: [MadeState; 4] = [
        MadeState::Written,
        MadeState::Rewritten,
        MadeState::Unchanged,
        MadeState::Removed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MadeState::Written => "written",
            MadeState::Rewritten => "rewritten",
            MadeState::Unchanged => "unchanged",
            MadeState::Removed => "removed",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentMade {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub state: MadeState,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentExpected {
    pub kind: String,
    pub name: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentProblem {
    pub kind: String,
    pub text: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentPayload {
    pub path: String,
    pub schema: String,
}

/// A ready-made conforming answer: the reserved keys, and the package's own beside them in `own`.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Answer<T = ()> {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default)]
    pub made: Vec<AgentMade>,
    #[serde(default)]
    pub expected: Vec<AgentExpected>,
    #[serde(default)]
    pub problems: Vec<AgentProblem>,
    #[serde(default)]
    pub payloads: Vec<AgentPayload>,
    #[serde(flatten)]
    pub own: T,
}

impl<T: Serialize + Send + 'static> AgentAnswer for Answer<T> {}
