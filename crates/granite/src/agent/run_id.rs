use std::any::TypeId;
use std::fmt;

/// One run of an agent: unique in its handle, keyed by what the agent made of its input.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AgentRunId {
    pub raw: String,
    pub key: String,
    /// The run that spawned this one, when one did.
    pub parent: Option<String>,
}

impl AgentRunId {
    pub const DEFAULT_KEY: &'static str = "default";

    pub fn new(raw: impl Into<String>, key: impl Into<String>, parent: Option<String>) -> Self {
        AgentRunId {
            raw: raw.into(),
            key: key.into(),
            parent,
        }
    }
}

impl fmt::Display for AgentRunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// The run the current task belongs to, so `perform` from inside a run on its own key is refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RunScope {
    pub run: AgentRunId,
    pub agent: TypeId,
}

tokio::task_local! {
    pub(crate) static CURRENT_RUN: RunScope;
}

impl RunScope {
    /// The run whose task is calling, when there is one.
    pub fn current() -> Option<RunScope> {
        CURRENT_RUN.try_with(|scope| scope.clone()).ok()
    }
}
