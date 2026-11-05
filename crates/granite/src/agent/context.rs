//! What a run computes with and how it reaches the world.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use super::{AgentHost, AgentLifecycle, AgentRunId, ChildFrame, GraniteAgent};

/// Whether a run was asked to stop; a child's flag also reads its parent's.
#[derive(Default)]
pub struct CancelFlag {
    set: AtomicBool,
    notify: Notify,
    parent: Option<Arc<CancelFlag>>,
}

impl CancelFlag {
    pub fn new() -> Arc<CancelFlag> {
        Arc::new(CancelFlag::default())
    }

    /// A flag that is set when `parent` is.
    pub fn child_of(parent: &Arc<CancelFlag>) -> Arc<CancelFlag> {
        Arc::new(CancelFlag {
            set: AtomicBool::new(false),
            notify: Notify::new(),
            parent: Some(parent.clone()),
        })
    }

    pub fn set(&self) {
        self.set.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_set(&self) -> bool {
        self.set.load(Ordering::SeqCst)
            || self.parent.as_ref().is_some_and(|parent| parent.is_set())
    }

    /// Returns once the flag, or its parent's, is set.
    pub async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_set() {
                return;
            }
            match &self.parent {
                Some(parent) => {
                    tokio::select! {
                        _ = notified => {}
                        _ = Box::pin(parent.wait()) => {}
                    }
                }
                None => notified.await,
            }
        }
    }
}

/// A run's id, its compute, its host, cancellation, and the way its events reach the writer.
pub struct AgentContext<C, E> {
    pub run: AgentRunId,
    pub compute: C,
    /// What the app that started the run offers it. A child inherits its parent's.
    pub host: AgentHost,
    cancel: Arc<CancelFlag>,
    sink: Arc<dyn Fn(E) + Send + Sync>,
}

impl<C: Clone, E> Clone for AgentContext<C, E> {
    fn clone(&self) -> Self {
        AgentContext {
            run: self.run.clone(),
            compute: self.compute.clone(),
            host: self.host.clone(),
            cancel: self.cancel.clone(),
            sink: self.sink.clone(),
        }
    }
}

impl<C, E> AgentContext<C, E>
where
    C: Clone + Send + Sync + 'static,
    E: Send + 'static,
{
    pub fn new(
        run: AgentRunId,
        compute: C,
        cancel: Arc<CancelFlag>,
        sink: impl Fn(E) + Send + Sync + 'static,
        host: AgentHost,
    ) -> Self {
        AgentContext {
            run,
            compute,
            host,
            cancel,
            sink: Arc::new(sink),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_set()
    }

    /// Returns once the run is asked to stop: for `tokio::select!` beside long work.
    pub async fn cancelled(&self) {
        self.cancel.wait().await
    }

    /// The flag behind [`AgentContext::is_cancelled`], for work handed to another thread.
    pub fn cancel_flag(&self) -> Arc<CancelFlag> {
        self.cancel.clone()
    }

    /// Delivers `event` into this run's stream.
    pub fn deliver(&self, event: E) {
        (self.sink)(event)
    }

    /// A line of progress, through the host.
    pub fn note(&self, line: impl AsRef<str>) {
        self.host.note(line.as_ref())
    }

    /// The same context, offered `host`: how a package call's host reaches the run the engine made.
    pub fn hosted(mut self, host: AgentHost) -> Self {
        self.host = host;
        self
    }

    /// Runs `child` and returns its frames, mapped into this run's events, for this run to deliver in its
    /// own order. Cancelling this run cancels the child; dropping the [`ChildRun`] aborts it.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn<Child: GraniteAgent>(
        &self,
        child: Child,
        input: Child::Input,
        compute: Child::Compute,
        id: &str,
        kind: &str,
        prompt: &str,
        map: impl Fn(ChildFrame<Child::Event>) -> Option<E> + Send + Sync + 'static,
    ) -> ChildRun<E> {
        let run = AgentRunId::new(id, Child::key(&input), Some(self.run.raw.clone()));
        let (tx, rx) = mpsc::unbounded_channel();
        let map = Arc::new(map);
        if let Some(announced) = map(ChildFrame::Started {
            run: run.clone(),
            kind: kind.to_owned(),
            prompt: prompt.to_owned(),
        }) {
            let _ = tx.send(announced);
        }
        let flag = CancelFlag::child_of(&self.cancel);
        let host = self.host.clone();
        let task = tokio::spawn(async move {
            let sink = {
                let (tx, map, run) = (tx.clone(), map.clone(), run.clone());
                move |event| {
                    if let Some(mapped) = map(ChildFrame::Event(run.clone(), event)) {
                        let _ = tx.send(mapped);
                    }
                }
            };
            let context = AgentContext::new(run.clone(), compute, flag.clone(), sink, host);
            let ended = match child.run(input, context).await {
                _ if flag.is_set() => AgentLifecycle::Cancelled,
                Ok(()) => AgentLifecycle::Finished,
                Err(error) => AgentLifecycle::Failed(error.to_string()),
            };
            if let Some(mapped) = map(ChildFrame::Finished(run, ended)) {
                let _ = tx.send(mapped);
            }
        });
        ChildRun { rx, task }
    }
}

/// A spawned child's frames, mapped into the parent's events. Dropping it aborts the child.
pub struct ChildRun<E> {
    rx: mpsc::UnboundedReceiver<E>,
    task: JoinHandle<()>,
}

impl<E> ChildRun<E> {
    /// The child's next mapped frame; none once it has ended.
    pub async fn next(&mut self) -> Option<E> {
        self.rx.recv().await
    }
}

impl<E> Drop for ChildRun<E> {
    fn drop(&mut self) {
        self.task.abort()
    }
}
