//! What a send does: a reducer folds its payload into the state on main, then its effect runs.
//!
//! PIN: A SEND COMMITS ONE TURN LATER. Every send is a deferred turn on main (clone → reduce → commit →
//! effect spawned), so a send from any thread lands in order and never races a commit.
//! PIN: REDUCE NEVER DISPATCHES. `reduce` gets the state and the payload, nothing to send with; what
//! follows a write is the effect's, after the commit, so a chained send can never clobber it.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    /// At most one send each `every`; with `latest`, the last one held back is sent when the window ends.
    Throttle { every: Duration, latest: bool },
}

struct Timing<P> {
    generation: u64,
    pending: Option<P>,
    fired: Option<Instant>,
    scheduled: bool,
}

/// A reducer bound to its center: Swift's `@Event`. Clones send to the same reducer.
pub struct Event<R: GraniteReducer> {
    center: Center<R::State>,
    reducer: Arc<R>,
    delivery: Delivery,
    timing: Arc<Mutex<Timing<R::Payload>>>,
}

impl<R: GraniteReducer> Clone for Event<R> {
    fn clone(&self) -> Self {
        Event {
            center: self.center.clone(),
            reducer: self.reducer.clone(),
            delivery: self.delivery,
            timing: self.timing.clone(),
        }
    }
}

impl<R: GraniteReducer> Event<R> {
    pub fn new(center: &Center<R::State>, reducer: R) -> Self {
        Event {
            center: center.clone(),
            reducer: Arc::new(reducer),
            delivery: Delivery::Immediate,
            timing: Arc::new(Mutex::new(Timing {
                generation: 0,
                pending: None,
                fired: None,
                scheduled: false,
            })),
        }
    }

    pub fn debounced(mut self, quiet: Duration) -> Self {
        self.delivery = Delivery::Debounce(quiet);
        self
    }

    pub fn throttled(mut self, every: Duration, latest: bool) -> Self {
        self.delivery = Delivery::Throttle { every, latest };
        self
    }

    /// Sends the payload's default each time the center reaches `when` (Swift's `@Event(.onAppear)`).
    pub fn lifecycle(self, when: Lifecycle) -> Self
    where
        R::Payload: Default,
    {
        let event = self.clone();
        self.center
            .on_lifecycle(when, move || event.send(R::Payload::default()));
        self
    }

    pub fn center(&self) -> &Center<R::State> {
        &self.center
    }

    /// Sends `payload`, from any thread, by the event's delivery.
    pub fn send(&self, payload: R::Payload) {
        match self.delivery {
            Delivery::Immediate => self.turn(payload),
            Delivery::Debounce(quiet) => {
                let generation = {
                    let mut timing = self.timing.lock().unwrap();
                    timing.generation += 1;
                    timing.pending = Some(payload);
                    timing.generation
                };
                let event = self.clone();
                self.center.main().spawn_work(async move {
                    tokio::time::sleep(quiet).await;
                    let payload = {
                        let mut timing = event.timing.lock().unwrap();
                        if timing.generation == generation {
                            timing.pending.take()
                        } else {
                            None
                        }
                    };
                    if let Some(payload) = payload {
                        event.turn(payload)
                    }
                });
            }
            Delivery::Throttle { every, latest } => {
                let now = Instant::now();
                let mut timing = self.timing.lock().unwrap();
                let open = timing
                    .fired
                    .is_none_or(|fired| now.duration_since(fired) >= every);
                if open {
                    timing.fired = Some(now);
                    drop(timing);
                    return self.turn(payload);
                }
                if !latest {
                    return;
                }
                timing.pending = Some(payload);
                if timing.scheduled {
                    return;
                }
                timing.scheduled = true;
                let wait = every.saturating_sub(now.duration_since(timing.fired.unwrap_or(now)));
                drop(timing);
                let event = self.clone();
                self.center.main().spawn_work(async move {
                    tokio::time::sleep(wait).await;
                    let payload = {
                        let mut timing = event.timing.lock().unwrap();
                        timing.scheduled = false;
                        timing.fired = Some(Instant::now());
                        timing.pending.take()
                    };
                    if let Some(payload) = payload {
                        event.turn(payload)
                    }
                });
            }
        }
    }

    /// Sends the payload's default.
    pub fn send_default(&self)
    where
        R::Payload: Default,
    {
        self.send(R::Payload::default())
    }

    fn turn(&self, payload: R::Payload) {
        let center = self.center.clone();
        let reducer = self.reducer.clone();
        self.center.main().enqueue(move || {
            let mut state = center.state();
            reducer.reduce(&mut state, &payload);
            center.commit(state);
            let committed = center.state();
            reducer.effect(&committed, payload).spawn(center.main());
        });
    }
}
