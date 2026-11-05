//! The engine behind [`super::Agent`]: Swift's `AgentRunContainer`, line for line where Rust allows.
//!
//! Starts runs, applies the concurrency policy, queues each run's events and drains them on main into one
//! commit, drops the events of a run that was superseded, feeds the taps a host reads, and settles the
//! waits `perform` and `wait` open.
//!
//! PIN: EVERY ENTRY IS ON MAIN — `start`, `cancel_key`, `drain` — as a Granite send is; the handle hops
//! before calling. A run's task only ever enqueues.
//! PIN: A WAIT SETTLES AFTER THE STATE IS WRITTEN. A finished run's waiters resume in the drain that
//! committed its lifecycle, after its last events; a cancelled one's after the `Cancelled` commit.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::run_id::CURRENT_RUN;
use super::{
    AgentConcurrency, AgentContext, AgentHost, AgentLifecycle, AgentOutcome, AgentRunError,
    AgentRunId, AgentStart, CancelFlag, GraniteAgent, RunScope,
};
use crate::{BoxError, BoxFuture, Center};

pub(crate) type Resolver<A> = Arc<
    dyn Fn(
            &<A as GraniteAgent>::Input,
        ) -> BoxFuture<'static, Result<<A as GraniteAgent>::Compute, BoxError>>
        + Send
        + Sync,
>;

enum Frame<E> {
    Event(E),
    Lifecycle(AgentLifecycle),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reason {
    Cancelled,
    Superseded,
}

struct LiveRun<A: GraniteAgent> {
    id: AgentRunId,
    task: Option<JoinHandle<()>>,
    cancel: Arc<CancelFlag>,
    context: Option<AgentContext<A::Compute, A::Event>>,
    /// Inputs merged before the run had its compute: handed to it once it does.
    pending_merges: Vec<A::Input>,
}

type Settled = Result<AgentOutcome, AgentRunError>;

/// Who waits on a run: how many opened a wait, who is suspended, and how it ended once it did.
#[derive(Default)]
struct Slot {
    openers: usize,
    waiters: HashMap<u64, oneshot::Sender<Settled>>,
    result: Option<Settled>,
}

struct Inner<A: GraniteAgent> {
    runs: HashMap<String, LiveRun<A>>,
    waiting: Vec<(AgentRunId, A::Input)>,
    frames: HashMap<String, Vec<Frame<A::Event>>>,
    frames_seen: HashMap<String, usize>,
    drains: HashSet<String>,
    taps: HashMap<String, Vec<mpsc::UnboundedSender<A::Event>>>,
    slots: HashMap<String, Slot>,
    counter: u64,
    tokens: u64,
    resolver: Option<Resolver<A>>,
}

pub(crate) struct RunContainer<A: GraniteAgent> {
    pub(crate) center: Center<A::State>,
    pub(crate) label: String,
    agent: Arc<A>,
    concurrency: AgentConcurrency,
    inner: Mutex<Inner<A>>,
}

/// A type's name without its path or generics: `PackageRun` for `granite_agents::package::PackageRun<Hello>`.
pub(crate) fn short_name<T: ?Sized>() -> String {
    let full = std::any::type_name::<T>();
    let bare = full.split('<').next().unwrap_or(full);
    bare.rsplit("::").next().unwrap_or(bare).to_owned()
}

impl<A: GraniteAgent> RunContainer<A> {
    pub(crate) fn new(center: &Center<A::State>, concurrency: AgentConcurrency) -> Arc<Self> {
        Arc::new(RunContainer {
            center: center.clone(),
            label: short_name::<A>(),
            agent: Arc::new(A::default()),
            concurrency,
            inner: Mutex::new(Inner {
                runs: HashMap::new(),
                waiting: Vec::new(),
                frames: HashMap::new(),
                frames_seen: HashMap::new(),
                drains: HashSet::new(),
                taps: HashMap::new(),
                slots: HashMap::new(),
                counter: 0,
                tokens: 0,
                resolver: None,
            }),
        })
    }

    pub(crate) fn attach(&self, resolver: Resolver<A>) {
        self.inner.lock().unwrap().resolver = Some(resolver);
    }

    pub(crate) fn is_live(&self, key: &str) -> bool {
        self.inner.lock().unwrap().runs.contains_key(key)
    }

    pub(crate) fn live_keys(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner
            .runs
            .keys()
            .cloned()
            .chain(inner.waiting.iter().map(|(run, _)| run.key.clone()))
            .collect()
    }

    fn mint(&self, key: &str) -> AgentRunId {
        let mut inner = self.inner.lock().unwrap();
        inner.counter += 1;
        AgentRunId::new(format!("{}-{}", self.label, inner.counter), key, None)
    }

    // MARK: runs (main)

    /// Main. Live runs commit `Cancelled`, and so do runs still waiting for a lane.
    pub(crate) fn cancel_prefix(self: &Arc<Self>, prefix: &str) {
        let (live, dropped) = {
            let mut inner = self.inner.lock().unwrap();
            let dropped: Vec<AgentRunId> = inner
                .waiting
                .iter()
                .filter(|(run, _)| run.key.starts_with(prefix))
                .map(|(run, _)| run.clone())
                .collect();
            inner
                .waiting
                .retain(|(run, _)| !run.key.starts_with(prefix));
            let live: Vec<String> = inner
                .runs
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect();
            (live, dropped)
        };
        for run in dropped {
            self.commit_lifecycle(AgentLifecycle::Cancelled, &run);
            self.close_taps(&run.key);
            self.settle(&run, Err(AgentRunError::Cancelled));
        }
        for key in live {
            self.cancel_key(&key, false);
        }
    }

    /// Main. Drops one run still waiting for a lane: it ends as one cancelled live would.
    pub(crate) fn cancel_queued(self: &Arc<Self>, run: &AgentRunId) {
        let dropped = {
            let mut inner = self.inner.lock().unwrap();
            match inner.waiting.iter().position(|(waiting, _)| waiting == run) {
                Some(index) => {
                    inner.waiting.remove(index);
                    true
                }
                None => false,
            }
        };
        if !dropped {
            return;
        }
        self.commit_lifecycle(AgentLifecycle::Cancelled, run);
        self.close_taps(&run.key);
        self.settle(run, Err(AgentRunError::Cancelled));
    }

    /// Main.
    pub(crate) fn start(self: &Arc<Self>, input: A::Input) -> AgentStart {
        let key = A::key(&input);
        let live = self
            .inner
            .lock()
            .unwrap()
            .runs
            .get(&key)
            .map(|run| run.id.clone());
        if let Some(live) = live {
            match self.concurrency {
                AgentConcurrency::Supersede => self.cancel_key(&key, true),
                AgentConcurrency::Drop => return AgentStart::Dropped,
                AgentConcurrency::Merge => {
                    let mut inner = self.inner.lock().unwrap();
                    let run = inner
                        .runs
                        .get_mut(&key)
                        .expect("granite: a live run vanished under the lock");
                    if let Some(context) = run.context.clone() {
                        drop(inner);
                        let agent = self.agent.clone();
                        let into = live.clone();
                        self.center
                            .main()
                            .spawn_work(async move { agent.merge(input, &into, &context).await });
                    } else {
                        run.pending_merges.push(input);
                    }
                    return AgentStart::Merged(live);
                }
                // The same key twice is the same run asked for again: the newer one replaces it.
                AgentConcurrency::Parallel { .. } => self.cancel_key(&key, true),
            }
        }
        if let AgentConcurrency::Parallel { .. } = self.concurrency {
            let replaced = {
                let mut inner = self.inner.lock().unwrap();
                inner
                    .waiting
                    .iter()
                    .position(|(run, _)| run.key == key)
                    .map(|index| inner.waiting.remove(index).0)
            };
            if let Some(replaced) = replaced {
                self.commit_lifecycle(AgentLifecycle::Cancelled, &replaced);
                self.close_taps(&key);
                self.settle(&replaced, Err(AgentRunError::Superseded(replaced.clone())));
            }
        }
        let run = self.mint(&key);
        if let AgentConcurrency::Parallel { max: Some(max) } = self.concurrency {
            let mut inner = self.inner.lock().unwrap();
            if inner.runs.len() >= max {
                inner.waiting.push((run.clone(), input));
                return AgentStart::Queued(run);
            }
        }
        self.launch(input, run.clone());
        AgentStart::Started(run)
    }

    /// Main.
    pub(crate) fn cancel_key(self: &Arc<Self>, key: &str, superseded: bool) {
        let reason = if superseded {
            Reason::Superseded
        } else {
            Reason::Cancelled
        };
        let live = {
            let mut inner = self.inner.lock().unwrap();
            let Some(live) = inner.runs.remove(key) else {
                return;
            };
            // Said now, on main, before any late event could: the run is over for the state and for
            // whoever was listening. Events it still enqueues are dropped (`drain` checks).
            inner.frames.remove(&live.id.raw);
            inner.frames_seen.remove(&live.id.raw);
            live
        };
        live.cancel.set();
        if let Some(task) = &live.task {
            task.abort();
        }
        self.commit_lifecycle(AgentLifecycle::Cancelled, &live.id);
        self.close_taps(key);
        let error = match reason {
            Reason::Superseded => AgentRunError::Superseded(live.id.clone()),
            Reason::Cancelled => AgentRunError::Cancelled,
        };
        self.settle(&live.id, Err(error));
        self.start_waiting();
    }

    fn launch(self: &Arc<Self>, input: A::Input, run: AgentRunId) {
        let cancel = CancelFlag::new();
        let resolver = {
            let mut inner = self.inner.lock().unwrap();
            inner.runs.insert(
                run.key.clone(),
                LiveRun {
                    id: run.clone(),
                    task: None,
                    cancel: cancel.clone(),
                    context: None,
                    pending_merges: Vec::new(),
                },
            );
            inner.resolver.clone()
        };
        self.commit_lifecycle(AgentLifecycle::Started, &run);
        let this = self.clone();
        let scope = RunScope {
            run: run.clone(),
            agent: TypeId::of::<A>(),
        };
        let id = run.clone();
        let task = self
            .center
            .main()
            .spawn_work(CURRENT_RUN.scope(scope, async move {
                let deliver = {
                    let (this, run) = (this.clone(), run.clone());
                    move |event| this.enqueue(Frame::Event(event), &run)
                };
                let outcome = match resolver {
                    None => AgentLifecycle::Failed(
                        AgentRunError::NoCompute(this.label.clone()).to_string(),
                    ),
                    Some(resolver) => match resolver(&input).await {
                        Err(_) if cancel.is_set() => AgentLifecycle::Cancelled,
                        Err(error) => AgentLifecycle::Failed(error.to_string()),
                        Ok(compute) => {
                            let context = AgentContext::new(
                                run.clone(),
                                compute,
                                cancel.clone(),
                                deliver,
                                AgentHost::none(),
                            );
                            for merged in this.adopt(&run, &context) {
                                let (agent, context, into) =
                                    (this.agent.clone(), context.clone(), run.clone());
                                tokio::spawn(
                                    async move { agent.merge(merged, &into, &context).await },
                                );
                            }
                            match this.agent.run(input, context).await {
                                _ if cancel.is_set() => AgentLifecycle::Cancelled,
                                Ok(()) => AgentLifecycle::Finished,
                                Err(error) => AgentLifecycle::Failed(error.to_string()),
                            }
                        }
                    },
                };
                this.enqueue(Frame::Lifecycle(outcome), &run);
            }));
        let mut inner = self.inner.lock().unwrap();
        match inner.runs.get_mut(&id.key) {
            Some(live) if live.id == id => live.task = Some(task),
            _ => task.abort(),
        }
    }

    /// The run has its compute: keeps its context for later merges, and hands back those that came first.
    fn adopt(
        &self,
        run: &AgentRunId,
        context: &AgentContext<A::Compute, A::Event>,
    ) -> Vec<A::Input> {
        let mut inner = self.inner.lock().unwrap();
        match inner.runs.get_mut(&run.key) {
            Some(live) if live.id == *run => {
                live.context = Some(context.clone());
                std::mem::take(&mut live.pending_merges)
            }
            _ => Vec::new(),
        }
    }

    fn start_waiting(self: &Arc<Self>) {
        let AgentConcurrency::Parallel { max } = self.concurrency else {
            return;
        };
        let next = {
            let mut inner = self.inner.lock().unwrap();
            let room = max.is_none_or(|max| inner.runs.len() < max);
            if room && !inner.waiting.is_empty() {
                Some(inner.waiting.remove(0))
            } else {
                None
            }
        };
        if let Some((run, input)) = next {
            self.launch(input, run);
        }
    }

    // MARK: events (any thread in, main out)

    fn enqueue(self: &Arc<Self>, frame: Frame<A::Event>, run: &AgentRunId) {
        let schedule = {
            let mut inner = self.inner.lock().unwrap();
            inner.frames.entry(run.raw.clone()).or_default().push(frame);
            inner.drains.insert(run.raw.clone())
        };
        if schedule {
            let (this, run) = (self.clone(), run.clone());
            self.center.main().enqueue(move || this.drain(&run));
        }
    }

    fn drain(self: &Arc<Self>, run: &AgentRunId) {
        let pending = {
            let mut inner = self.inner.lock().unwrap();
            inner.drains.remove(&run.raw);
            let pending = inner.frames.remove(&run.raw).unwrap_or_default();
            // An event of a run that was superseded or cancelled is dropped: its state is gone.
            if pending.is_empty() || inner.runs.get(&run.key).is_none_or(|live| live.id != *run) {
                return;
            }
            pending
        };
        let mut events: Vec<A::Event> = Vec::new();
        let mut seen = 0;
        let mut ended = None;
        for frame in pending {
            match frame {
                Frame::Event(event) => {
                    seen += 1;
                    match events.last().and_then(|last| A::coalesce(last, &event)) {
                        Some(one) => *events.last_mut().expect("a last event") = one,
                        None => events.push(event),
                    }
                }
                Frame::Lifecycle(lifecycle) => ended = Some(lifecycle),
            }
        }
        let taps = {
            let mut inner = self.inner.lock().unwrap();
            *inner.frames_seen.entry(run.raw.clone()).or_default() += seen;
            inner.taps.get(&run.key).cloned().unwrap_or_default()
        };
        for event in &events {
            for tap in &taps {
                let _ = tap.send(event.clone());
            }
        }
        let mut state = self.center.state();
        for event in events {
            self.agent.reduce(&mut state, event, run);
        }
        if let Some(ended) = &ended {
            self.agent.reduce_lifecycle(&mut state, ended, run);
        }
        self.center.commit(state);
        let Some(ended) = ended else { return };
        let count = {
            let mut inner = self.inner.lock().unwrap();
            if inner.runs.get(&run.key).is_some_and(|live| live.id == *run) {
                inner.runs.remove(&run.key);
            }
            inner.frames_seen.remove(&run.raw).unwrap_or(0)
        };
        self.close_taps(&run.key);
        self.start_waiting();
        let result = match ended {
            AgentLifecycle::Finished | AgentLifecycle::Started => Ok(AgentOutcome {
                run: run.clone(),
                frames: count,
            }),
            AgentLifecycle::Failed(message) => Err(AgentRunError::Failed(run.clone(), message)),
            AgentLifecycle::Cancelled => Err(AgentRunError::Cancelled),
        };
        self.settle(run, result);
    }

    fn commit_lifecycle(&self, lifecycle: AgentLifecycle, run: &AgentRunId) {
        let mut state = self.center.state();
        self.agent.reduce_lifecycle(&mut state, &lifecycle, run);
        self.center.commit(state);
    }

    // MARK: waits

    /// Main. Opens a wait on `run` so a settle that lands before the waiter suspends is kept.
    fn open(&self, run: &AgentRunId) {
        self.inner
            .lock()
            .unwrap()
            .slots
            .entry(run.raw.clone())
            .or_default()
            .openers += 1;
    }

    fn settle(&self, run: &AgentRunId, result: Settled) {
        let waiters = {
            let mut inner = self.inner.lock().unwrap();
            let Some(slot) = inner.slots.get_mut(&run.raw) else {
                return;
            };
            let waiters: Vec<oneshot::Sender<Settled>> =
                slot.waiters.drain().map(|(_, waiter)| waiter).collect();
            slot.openers = slot.openers.saturating_sub(waiters.len());
            slot.result = Some(result.clone());
            if slot.openers == 0 {
                inner.slots.remove(&run.raw);
            }
            waiters
        };
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    /// Suspends on an opened wait until the run settles. Dropped before it settles, it stops waiting, and,
    /// when `owned`, cancels the run it started.
    async fn suspend(self: &Arc<Self>, run: &AgentRunId, owned: bool) -> Settled {
        let (token, receiver) = {
            let mut inner = self.inner.lock().unwrap();
            inner.tokens += 1;
            let token = inner.tokens;
            let Some(slot) = inner.slots.get_mut(&run.raw) else {
                return Err(AgentRunError::UnknownRun(run.clone()));
            };
            if let Some(result) = slot.result.clone() {
                slot.openers = slot.openers.saturating_sub(1);
                if slot.openers == 0 {
                    inner.slots.remove(&run.raw);
                }
                return result;
            }
            let (tx, rx) = oneshot::channel();
            slot.waiters.insert(token, tx);
            (token, rx)
        };
        let mut guard = WaitGuard {
            container: self.clone(),
            run: run.clone(),
            token,
            owned,
            armed: true,
        };
        let result = receiver.await.unwrap_or(Err(AgentRunError::Cancelled));
        guard.armed = false;
        result
    }

    pub(crate) async fn perform(self: &Arc<Self>, input: A::Input) -> Settled {
        if let Some(scope) = RunScope::current()
            && scope.agent == TypeId::of::<A>()
            && scope.run.key == A::key(&input)
        {
            return Err(AgentRunError::Reentrant(scope.run));
        }
        // One main turn opens the wait and starts the run, so no event can land between the two.
        let this = self.clone();
        let (run, owned) = self
            .center
            .main()
            .call(move || match this.start(input) {
                AgentStart::Started(run) | AgentStart::Queued(run) => {
                    this.open(&run);
                    Ok((run, true))
                }
                AgentStart::Merged(run) => {
                    this.open(&run);
                    Ok((run, false))
                }
                AgentStart::Dropped => Err(AgentRunError::Dropped(this.label.clone())),
            })
            .await?;
        self.suspend(&run, owned).await
    }

    pub(crate) async fn wait(self: &Arc<Self>, run: &AgentRunId) -> Settled {
        let this = self.clone();
        let asked = run.clone();
        let known = self
            .center
            .main()
            .call(move || {
                let known = {
                    let inner = this.inner.lock().unwrap();
                    inner.slots.contains_key(&asked.raw)
                        || inner
                            .runs
                            .get(&asked.key)
                            .is_some_and(|live| live.id == asked)
                        || inner.waiting.iter().any(|(waiting, _)| *waiting == asked)
                };
                if known {
                    this.open(&asked);
                }
                known
            })
            .await;
        if !known {
            return Err(AgentRunError::UnknownRun(run.clone()));
        }
        self.suspend(run, false).await
    }

    pub(crate) async fn wait_prefix(self: &Arc<Self>, prefix: &str) {
        let this = self.clone();
        let prefix = prefix.to_owned();
        let opened = self
            .center
            .main()
            .call(move || {
                let runs: Vec<AgentRunId> = {
                    let inner = this.inner.lock().unwrap();
                    inner
                        .runs
                        .values()
                        .map(|live| live.id.clone())
                        .chain(inner.waiting.iter().map(|(run, _)| run.clone()))
                        .filter(|run| run.key.starts_with(&prefix))
                        .collect()
                };
                for run in &runs {
                    this.open(run);
                }
                runs
            })
            .await;
        for run in opened {
            let _ = self.suspend(&run, false).await;
        }
    }

    // MARK: taps

    pub(crate) fn events(&self, key: &str) -> mpsc::UnboundedReceiver<A::Event> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.inner
            .lock()
            .unwrap()
            .taps
            .entry(key.to_owned())
            .or_default()
            .push(tx);
        rx
    }

    fn close_taps(&self, key: &str) {
        self.inner.lock().unwrap().taps.remove(key);
    }
}

struct WaitGuard<A: GraniteAgent> {
    container: Arc<RunContainer<A>>,
    run: AgentRunId,
    token: u64,
    owned: bool,
    armed: bool,
}

impl<A: GraniteAgent> Drop for WaitGuard<A> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // This waiter only: a dropped caller stops waiting; whether the run stops is `owned`'s.
        {
            let mut inner = self.container.inner.lock().unwrap();
            if let Some(slot) = inner.slots.get_mut(&self.run.raw) {
                if slot.waiters.remove(&self.token).is_some() {
                    slot.openers = slot.openers.saturating_sub(1);
                }
                if slot.openers == 0 && slot.waiters.is_empty() {
                    inner.slots.remove(&self.run.raw);
                }
            }
        }
        if self.owned {
            let (container, run) = (self.container.clone(), self.run.clone());
            self.container.center.main().enqueue(move || {
                let live = container
                    .inner
                    .lock()
                    .unwrap()
                    .runs
                    .get(&run.key)
                    .is_some_and(|live| live.id == run);
                if live {
                    container.cancel_key(&run.key, false)
                } else {
                    container.cancel_queued(&run)
                }
            });
        }
    }
}
