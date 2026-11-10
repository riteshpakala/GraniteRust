//! What a send does: a reducer folds its payload into the state on main, then its effect runs.
//!
//! PIN: A SEND COMMITS ONE TURN LATER. Every send is a deferred turn on main (clone → reduce → commit →
//! effect spawned), so a send from any thread lands in order and never races a commit.
//! PIN: REDUCE NEVER DISPATCHES. `reduce` gets the state and the payload, nothing to send with; what
//! follows a write is the effect's, after the commit, so a chained send can never clobber it.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::locked::Locked;
use crate::{BoxFuture, Center, GraniteState, Lifecycle};

/// A reducer: what one kind of send does to its center's state, and the work that follows.
pub trait GraniteReducer: Send + Sync + 'static {
    type State: GraniteState;
    /// What a send carries; `()` for none.
    type Payload: Send + 'static;

    /// Folds `payload` into `state`. Synchronous, on main.
    fn reduce(&self, state: &mut Self::State, payload: &Self::Payload);

    /// The work after the commit, given the committed state. Default: none.
    fn effect(&self, _state: &Self::State, _payload: Self::Payload) -> Effect {
        Effect::None
    }
}

/// The work a reducer hands off: spawned on main's runtime after the commit.
pub enum Effect {
    None,
    Run(BoxFuture<'static, ()>),
    Many(Vec<Effect>),
}

impl Effect {
    pub fn run(work: impl Future<Output = ()> + Send + 'static) -> Effect {
        Effect::Run(Box::pin(work))
    }

    /// Sends `payload` through `event`: a reducer handing off to a sibling, after its own commit.
    pub fn chain<R: GraniteReducer>(event: &Event<R>, payload: R::Payload) -> Effect {
        let event = event.clone();
        Effect::run(async move { event.send(payload) })
    }

    pub fn many(effects: impl IntoIterator<Item = Effect>) -> Effect {
        Effect::Many(effects.into_iter().collect())
    }

    fn spawn(self, main: &crate::Main) {
        match self {
            Effect::None => {}
            Effect::Run(work) => {
                main.spawn_work(work);
            }
            Effect::Many(effects) => effects.into_iter().for_each(|effect| effect.spawn(main)),
        }
    }
}

/// How sends reach the reducer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delivery {
    /// Every send.
    Immediate,
    /// The last send of a burst, once `0` has passed with none after it.
    Debounce(Duration),
    /// At most one send each `every`: the first at once, then one at the window's end for those inside
    /// it (the latest with `latest`, else the first).
    Throttle { every: Duration, latest: bool },
}

/// Where an event's sends stand: its delivery, and what its timers hold back.
struct Timing<P> {
    delivery: Delivery,
    generation: u64,
    pending: Option<P>,
    fired: Option<Instant>,
    scheduled: bool,
}

/// What a throttled send does next.
enum Next<P> {
    /// Turn it now: the window is open.
    Now(P),
    /// A timer turns the held-back payload after this long: when the window ends.
    After(Duration),
    /// A timer is already set.
    Wait,
}

/// What every clone of an event shares: its center, its reducer and where its sends stand.
struct Bound<R: GraniteReducer> {
    center: Center<R::State>,
    reducer: R,
    timing: Locked<Timing<R::Payload>>,
}

/// A reducer bound to its center: Swift's `@Event`. Clones send to the same reducer, by one delivery.
///
/// PIN: A TIMER DOES NOT KEEP ITS EVENT. A debounced or throttled payload waits in the event; the timer
/// holds the event weakly, so a payload still waiting when the last clone goes is dropped with it.
pub struct Event<R: GraniteReducer> {
    inner: Arc<Bound<R>>,
}

impl<R: GraniteReducer> Clone for Event<R> {
    fn clone(&self) -> Self {
        Event {
            inner: self.inner.clone(),
        }
    }
}

impl<R: GraniteReducer> Event<R> {
    pub fn new(center: &Center<R::State>, reducer: R) -> Self {
        Event {
            inner: Arc::new(Bound {
                center: center.clone(),
                reducer,
                timing: Locked::new(Timing {
                    delivery: Delivery::Immediate,
                    generation: 0,
                    pending: None,
                    fired: None,
                    scheduled: false,
                }),
            }),
        }
    }

    /// The last send of a burst, once `quiet` has passed with none after it. Every clone delivers so.
    pub fn debounced(self, quiet: Duration) -> Self {
        self.deliver(Delivery::Debounce(quiet))
    }

    /// At most one send each `every`: the first at once, and those that arrive inside the window coalesced
    /// into one at its end (the latest with `latest`, else the first). Every clone delivers so.
    pub fn throttled(self, every: Duration, latest: bool) -> Self {
        self.deliver(Delivery::Throttle { every, latest })
    }

    fn deliver(self, delivery: Delivery) -> Self {
        self.inner.timing.with(|timing| timing.delivery = delivery);
        self
    }

    /// Sends the payload's default each time the center reaches `when` (Swift's `@Event(.onAppear)`),
    /// for as long as the event is held: the center holds it weakly.
    pub fn lifecycle(self, when: Lifecycle) -> Self
    where
        R::Payload: Default,
    {
        self.inner
            .center
            .on_lifecycle_of(&self.inner, when, |bound| {
                Bound::send(bound, R::Payload::default())
            });
        self
    }

    pub fn center(&self) -> &Center<R::State> {
        &self.inner.center
    }

    /// Sends `payload`, from any thread, by the event's delivery.
    pub fn send(&self, payload: R::Payload) {
        Bound::send(&self.inner, payload)
    }

    /// Sends the payload's default.
    pub fn send_default(&self)
    where
        R::Payload: Default,
    {
        self.send(R::Payload::default())
    }
}

impl<R: GraniteReducer> Bound<R> {
    fn send(this: &Arc<Self>, payload: R::Payload) {
        let delivery = this.timing.with(|timing| timing.delivery);
        match delivery {
            Delivery::Immediate => this.turn(payload),
            Delivery::Debounce(quiet) => {
                // The payload it replaces is dropped after the lock.
                let (generation, replaced) = this.timing.with(|timing| {
                    timing.generation += 1;
                    (timing.generation, timing.pending.replace(payload))
                });
                drop(replaced);
                let bound = Arc::downgrade(this);
                this.center.main().spawn_work(async move {
                    tokio::time::sleep(quiet).await;
                    let Some(bound) = bound.upgrade() else { return };
                    let payload = bound.timing.with(|timing| {
                        if timing.generation == generation {
                            timing.pending.take()
                        } else {
                            None
                        }
                    });
                    if let Some(payload) = payload {
                        bound.turn(payload)
                    }
                });
            }
            Delivery::Throttle { every, latest } => {
                let now = Instant::now();
                // A payload replaced, or refused under `latest: false`, is dropped after the lock.
                let (next, dropped) = this.timing.with(|timing| {
                    let open = !timing.scheduled
                        && timing
                            .fired
                            .is_none_or(|fired| now.duration_since(fired) >= every);
                    if open {
                        timing.fired = Some(now);
                        return (Next::Now(payload), None);
                    }
                    let dropped = if latest || timing.pending.is_none() {
                        timing.pending.replace(payload)
                    } else {
                        Some(payload)
                    };
                    if timing.scheduled {
                        return (Next::Wait, dropped);
                    }
                    timing.scheduled = true;
                    let fired = timing.fired.unwrap_or(now);
                    (
                        Next::After(every.saturating_sub(now.duration_since(fired))),
                        dropped,
                    )
                });
                drop(dropped);
                match next {
                    Next::Now(payload) => this.turn(payload),
                    Next::Wait => {}
                    Next::After(wait) => {
                        let bound = Arc::downgrade(this);
                        this.center.main().spawn_work(async move {
                            tokio::time::sleep(wait).await;
                            let Some(bound) = bound.upgrade() else { return };
                            let payload = bound.timing.with(|timing| {
                                timing.scheduled = false;
                                let payload = timing.pending.take();
                                if payload.is_some() {
                                    timing.fired = Some(Instant::now());
                                }
                                payload
                            });
                            if let Some(payload) = payload {
                                bound.turn(payload)
                            }
                        });
                    }
                }
            }
        }
    }

    fn turn(self: &Arc<Self>, payload: R::Payload) {
        let bound = self.clone();
        self.center.main().enqueue(move || {
            let center = &bound.center;
            let mut state = center.state();
            bound.reducer.reduce(&mut state, &payload);
            center.commit(state);
            let committed = center.state();
            bound
                .reducer
                .effect(&committed, payload)
                .spawn(center.main());
        });
    }
}
