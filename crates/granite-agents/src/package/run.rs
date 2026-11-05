//! The `GraniteAgent` a package's calls are hosted as: every call a run of its own, keyed by the call's id, up
//! to the package's lanes at once; its events and lifecycle counted into the package center by the one writer.

use std::sync::{Arc, Mutex};

use granite::{AgentConcurrency, AgentLifecycle, AgentRunId, GraniteAgent};

use super::AgentPackage;
use crate::{AgentContext, AgentHost, BoxError};

/// One call of a package agent, and what the app that made it offers the run.
pub struct PackageCall<I> {
    pub id: String,
    pub input: I,
    pub host: AgentHost,
    pub(crate) thrown: Thrown,
}

/// The error a package's run returned, kept beside its call: the engine says a failure as a sentence, and a
/// refusal's kind would be lost in it.
#[derive(Clone, Default)]
pub(crate) struct Thrown(Arc<Mutex<Option<BoxError>>>);

impl Thrown {
    fn keep(&self, error: BoxError) {
        let mut kept = self.0.lock().unwrap();
        if kept.is_none() {
            *kept = Some(error);
        }
    }

    pub(crate) fn take(&self) -> Option<BoxError> {
        self.0.lock().unwrap().take()
    }
}

/// What the package center counts.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct PackageState {
    pub runs: u64,
    pub finished: u64,
    pub cancelled: u64,
    pub failed: u64,
    pub events: u64,
    pub last_failure: Option<String>,
}

pub struct PackageRun<P: AgentPackage> {
    package: P,
}

impl<P: AgentPackage> Default for PackageRun<P> {
    fn default() -> Self {
        PackageRun {
            package: P::default(),
        }
    }
}

impl<P: AgentPackage> GraniteAgent for PackageRun<P> {
    type State = PackageState;
    type Input = PackageCall<P::Input>;
    type Event = P::Event;
    type Compute = P::Compute;

    fn concurrency() -> AgentConcurrency {
        AgentConcurrency::Parallel {
            max: P::lanes().map(|lanes| lanes.max(1) as usize),
        }
    }

    fn key(input: &Self::Input) -> String {
        input.id.clone()
    }

    async fn run(
        &self,
        call: Self::Input,
        context: AgentContext<P::Compute, P::Event>,
    ) -> Result<(), BoxError> {
        let PackageCall {
            input,
            host,
            thrown,
            ..
        } = call;
        match self.package.run(input, context.hosted(host)).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let said = error.to_string();
                thrown.keep(error);
                Err(said.into())
            }
        }
    }

    fn reduce(&self, state: &mut PackageState, _event: P::Event, _run: &AgentRunId) {
        state.events += 1;
    }

    fn reduce_lifecycle(
        &self,
        state: &mut PackageState,
        lifecycle: &AgentLifecycle,
        _run: &AgentRunId,
    ) {
        match lifecycle {
            AgentLifecycle::Started => state.runs += 1,
            AgentLifecycle::Finished => state.finished += 1,
            AgentLifecycle::Cancelled => state.cancelled += 1,
            AgentLifecycle::Failed(message) => {
                state.failed += 1;
                state.last_failure = Some(message.clone());
            }
        }
    }
}
