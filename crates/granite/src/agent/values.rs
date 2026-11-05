use std::fmt;

use super::AgentRunId;

/// What a second send does while a run with the same key is live.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentConcurrency {
    /// Cancel the live run and start this one (the default).
    Supersede,
    /// Ignore this send.
    Drop,
    /// Hand the input to the live run ([`super::GraniteAgent::merge`]): steering a run under way.
    Merge,
    /// Run beside it, up to `max` at once; more wait their turn.
    Parallel { max: Option<usize> },
}

/// How a run begins and ends, as the writer reduces it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AgentLifecycle {
    Started,
    Finished,
    Cancelled,
    Failed(String),
}

impl fmt::Display for AgentLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentLifecycle::Started => f.write_str("started"),
            AgentLifecycle::Finished => f.write_str("finished"),
            AgentLifecycle::Cancelled => f.write_str("cancelled"),
            AgentLifecycle::Failed(message) => write!(f, "failed: {message}"),
        }
    }
}

/// How an awaited run ended, when it finished.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AgentOutcome {
    pub run: AgentRunId,
    /// The events the run streamed, counted before they were folded into commits.
    pub frames: usize,
}

/// What a send did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AgentStart {
    /// A run began.
    Started(AgentRunId),
    /// `Parallel` at its max: the run has its id now and begins when a lane frees.
    Queued(AgentRunId),
    /// `Merge`: the input went to the live run.
    Merged(AgentRunId),
    /// `Drop`: a run with this key is live; nothing happened.
    Dropped,
}

impl AgentStart {
    /// The run this send is part of, when it is part of one.
    pub fn run(&self) -> Option<&AgentRunId> {
        match self {
            AgentStart::Started(run) | AgentStart::Queued(run) | AgentStart::Merged(run) => {
                Some(run)
            }
            AgentStart::Dropped => None,
        }
    }
}

/// Why an awaited run gave no outcome.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AgentRunError {
    /// A send before `attach`: nothing to compute with.
    NoCompute(String),
    /// `Drop`: a run with this key is live, so this one never began.
    Dropped(String),
    /// A later send with the same key took this run's place.
    Superseded(AgentRunId),
    /// The run's lifecycle was `Failed` with this message.
    Failed(AgentRunId, String),
    /// A wait on a run that is neither live nor waiting: it ended before anyone opened a wait.
    UnknownRun(AgentRunId),
    /// `perform` from inside the run it would wait on.
    Reentrant(AgentRunId),
    /// The run was cancelled, or the wait was.
    Cancelled,
}

impl fmt::Display for AgentRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentRunError::NoCompute(agent) => {
                write!(
                    f,
                    "{agent} has no compute attached: call `agent.attach(|input| async {{ … }})` from the composition root"
                )
            }
            AgentRunError::Dropped(agent) => write!(
                f,
                "{agent} is already running for this key, and its policy drops a second send"
            ),
            AgentRunError::Superseded(run) => {
                write!(f, "{run} was replaced by a later send with the same key")
            }
            AgentRunError::Failed(run, message) => write!(f, "{run} failed: {message}"),
            AgentRunError::UnknownRun(run) => write!(
                f,
                "{run} is neither running nor waiting: open the wait in the turn that starts it"
            ),
            AgentRunError::Reentrant(run) => write!(
                f,
                "{run} cannot wait on itself: reach a child through `context.spawn`"
            ),
            AgentRunError::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for AgentRunError {}

/// What a spawned child's run says to its parent: announced, each event, and how it ended.
#[derive(Clone, Debug)]
pub enum ChildFrame<E> {
    Started {
        run: AgentRunId,
        kind: String,
        prompt: String,
    },
    Event(AgentRunId, E),
    Finished(AgentRunId, AgentLifecycle),
}
