//! The handle a service holds and hands out: Swift's `@Agent` and `AgentExecutable`.
//!
//! Two ways to start a run, the two a reducer has:
//! - [`Agent::send`]: the run begins; its events are reduced on later main turns by the agent's own `reduce`.
//! - [`Agent::perform`]: the run begins and the call returns once its lifecycle has been reduced and
//!   committed after its last events, so the state read next is complete. Dropping the future cancels a
//!   run it started or queued, never one it merged into.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::container::RunContainer;
use super::{AgentConcurrency, AgentOutcome, AgentRunError, AgentRunId, AgentStart, GraniteAgent};
use crate::{BoxError, Center, Lifecycle};

/// An agent bound to its center. Clones drive the same runs.
pub struct Agent<A: GraniteAgent> {
    container: Arc<RunContainer<A>>,
}

impl<A: GraniteAgent> Clone for Agent<A> {
    fn clone(&self) -> Self {
        Agent {
            container: self.container.clone(),
        }
    }
}

/// Every event of one run, as it streams and before it is reduced, until the run ends.
pub struct Tap<E>(mpsc::UnboundedReceiver<E>);

impl<E> Tap<E> {
    pub async fn next(&mut self) -> Option<E> {
        self.0.recv().await
    }

    /// Every event until the run ends.
    pub async fn collect(mut self) -> Vec<E> {
        let mut events = Vec::new();
        while let Some(event) = self.0.recv().await {
            events.push(event);
        }
        events
    }
}

impl<A: GraniteAgent> Agent<A> {
    /// The agent on `center`, under its own concurrency.
    pub fn new(center: &Center<A::State>) -> Self {
        Self::with_concurrency(center, A::concurrency())
    }

    /// The agent on `center` under `concurrency`: the declaration's policy wins over the agent's own.
    pub fn with_concurrency(center: &Center<A::State>, concurrency: AgentConcurrency) -> Self {
        Agent {
            container: RunContainer::new(center, concurrency),
        }
    }

    /// Starts a run with [`GraniteAgent::lifecycle_input`] each time the center reaches `when`.
    pub fn lifecycle(self, when: Lifecycle) -> Self {
        let container = self.container.clone();
        self.container.center.on_lifecycle(when, move || {
            if let Some(input) = A::lifecycle_input() {
                container.start(input);
            }
        });
        self
    }

    pub fn center(&self) -> &Center<A::State> {
        &self.container.center
    }

    /// The composition root supplies what a run computes with, from its input.
    pub fn attach<F, Fut>(&self, resolve: F)
    where
        F: Fn(&A::Input) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<A::Compute, BoxError>> + Send + 'static,
    {
        self.container
            .attach(Arc::new(move |input| Box::pin(resolve(input))));
    }

    /// Every run computes with `compute`.
    pub fn attach_compute(&self, compute: A::Compute) {
        self.attach(move |_| {
            let compute = compute.clone();
            async move { Ok(compute) }
        });
    }

    /// Starts a run (or merges, drops, queues, by the policy), from any thread.
    pub fn send(&self, input: A::Input) {
        let container = self.container.clone();
        self.container.center.main().run(move || {
            container.start(input);
        });
    }

    /// Starts a run and says what happened, with the run's id.
    pub async fn start(&self, input: A::Input) -> AgentStart {
        let container = self.container.clone();
        self.container
            .center
            .main()
            .call(move || container.start(input))
            .await
    }

    /// Starts the run (or merges into the live one) and returns once its lifecycle has been reduced and
    /// committed after its last events.
    pub async fn perform(&self, input: A::Input) -> Result<AgentOutcome, AgentRunError> {
        self.container.perform(input).await
    }

    /// Waits on a run someone else started; `UnknownRun` once it ended with no wait open.
    pub async fn wait(&self, run: &AgentRunId) -> Result<AgentOutcome, AgentRunError> {
        self.container.wait(run).await
    }

    /// Returns when every run whose key starts with `prefix`, live or waiting, has ended, however it ended.
    pub async fn wait_prefix(&self, prefix: &str) {
        self.container.wait_prefix(prefix).await
    }

    /// Cancels the live run with `key`, committing `Cancelled` at once.
    pub fn cancel(&self, key: &str) {
        let (container, key) = (self.container.clone(), key.to_owned());
        self.container
            .center
            .main()
            .run(move || container.cancel_key(&key, false));
    }

    pub fn cancel_default(&self) {
        self.cancel(AgentRunId::DEFAULT_KEY)
    }

    /// Cancels every run whose key starts with `prefix`, and drops those still waiting.
    pub fn cancel_prefix(&self, prefix: &str) {
        let (container, prefix) = (self.container.clone(), prefix.to_owned());
        self.container
            .center
            .main()
            .run(move || container.cancel_prefix(&prefix));
    }

    pub fn is_live(&self, key: &str) -> bool {
        self.container.is_live(key)
    }

    /// The keys of every run that is live or waiting for a lane.
    pub fn live_keys(&self) -> Vec<String> {
        self.container.live_keys()
    }

    /// Every event of the run with `key`, before it is reduced, until the run ends.
    pub fn events(&self, key: &str) -> Tap<A::Event> {
        Tap(self.container.events(key))
    }
}
