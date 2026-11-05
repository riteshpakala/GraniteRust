//! Agents: work that runs off main and streams what it finds, folded into a center's state by one writer.
//!
//! A [`GraniteAgent`] declares an input, the events a run streams, and what a run computes with. Its run
//! never touches state; each event is reduced into the center's state on main by the agent's own
//! `reduce`, keyed by run, so a late event of a superseded run is dropped. [`Agent`] is the handle a
//! service holds (Swift's `@Agent var name: Name.Agent`): `send`, `perform`, `cancel`, `events`.
//!
//! PIN: Granite owns durable state; the run owns compute. What a run computes with is attached by the
//! composition root ([`Agent::attach`]): a model session, a GPU pipeline, nothing the engine names.

mod container;
mod context;
mod executable;
mod host;
mod run_id;
mod values;

use std::future::Future;

pub use context::{AgentContext, CancelFlag, ChildRun};
pub use executable::{Agent, Tap};
pub use host::{AgentHost, AgentHostError, AgentHostModel};
pub use run_id::{AgentRunId, RunScope};
pub use values::{
    AgentConcurrency, AgentLifecycle, AgentOutcome, AgentRunError, AgentStart, ChildFrame,
};

use crate::{BoxError, GraniteState};

/// An agent, as a service declares one.
pub trait GraniteAgent: Default + Send + Sync + 'static {
    /// The state of the center its events are reduced into.
    type State: GraniteState;
    /// What a send carries: the ask, and which session or key it is for.
    type Input: Send + 'static;
    /// What a run streams.
    type Event: Clone + Send + 'static;
    /// What a run computes with, resolved by the composition root. Wrap heavy things in `Arc`.
    type Compute: Clone + Send + Sync + 'static;

    /// What a second send does while a run with the same key is live. Default: supersede.
    fn concurrency() -> AgentConcurrency {
        AgentConcurrency::Supersede
    }

    /// Which runs are the same: one key per agent by default.
    fn key(_input: &Self::Input) -> String {
        AgentRunId::DEFAULT_KEY.to_owned()
    }

    /// The input a run begun by the center's lifecycle starts with; none by default.
    fn lifecycle_input() -> Option<Self::Input> {
        None
    }

    /// Compute, off main. Emits with `context.deliver`; `context.is_cancelled()` is the cooperative check.
    fn run(
        &self,
        input: Self::Input,
        context: AgentContext<Self::Compute, Self::Event>,
    ) -> impl Future<Output = Result<(), BoxError>> + Send;

    /// The single writer: fresh state every drain, on main, keyed by run.
    fn reduce(&self, state: &mut Self::State, event: Self::Event, run: &AgentRunId);

    /// The run began, ended, was cancelled or failed.
    fn reduce_lifecycle(
        &self,
        _state: &mut Self::State,
        _lifecycle: &AgentLifecycle,
        _run: &AgentRunId,
    ) {
    }

    /// A send while live, under [`AgentConcurrency::Merge`].
    fn merge(
        &self,
        _input: Self::Input,
        _run: &AgentRunId,
        _context: &AgentContext<Self::Compute, Self::Event>,
    ) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Two queued events folded into one (text deltas), so main commits once per drain.
    fn coalesce(_a: &Self::Event, _b: &Self::Event) -> Option<Self::Event> {
        None
    }
}
