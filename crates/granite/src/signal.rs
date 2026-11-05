//! Fan-out of values to observers: Swift's `GraniteSignal`.
//!
//! PIN: DELIVERY IS SYNCHRONOUS, LOCK RELEASED. `send` copies the observer list under the lock and calls
//! each handler with it released, on the sender's thread, so a handler may send, observe or cancel on the
//! same signal. A handler cancelled by an earlier one in the same fan-out does not fire.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{Main, Subscription};

type Handler<T> = Arc<dyn Fn(T, Option<T>) + Send + Sync>;

struct Observer<T> {
    id: u64,
    handler: Mutex<Option<Handler<T>>>,
}

struct Core<T> {
    observers: Mutex<Vec<Arc<Observer<T>>>>,
    latest: Mutex<Option<T>>,
    next: AtomicU64,
    keep: Mutex<Vec<Subscription>>,
}

/// A throttle's window: when it last sent, the value held back, and whether a send is scheduled.
struct Window<T> {
    fired: Option<std::time::Instant>,
    held: Option<T>,
    scheduled: bool,
}

/// A stream of values with any number of observers. Clones share observers.
pub struct Signal<T: Clone + Send + Sync + 'static> {
    core: Arc<Core<T>>,
}

impl<T: Clone + Send + Sync + 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Signal {
            core: self.core.clone(),
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Default for Signal<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + Sync + 'static> Signal<T> {
    pub fn new() -> Self {
        Signal {
            core: Arc::new(Core {
                observers: Mutex::new(Vec::new()),
                latest: Mutex::new(None),
                next: AtomicU64::new(1),
                keep: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn seeded(value: T) -> Self {
        let signal = Self::new();
        *signal.core.latest.lock().unwrap() = Some(value);
        signal
    }

    /// Delivers `value` to every observer, in the order they observed.
    pub fn send(&self, value: T) {
        let previous = self.core.latest.lock().unwrap().replace(value.clone());
        let observers = self.core.observers.lock().unwrap().clone();
        for observer in observers {
            // Read per delivery, called with the lock released: a handler cancelled by an earlier one does
            // not fire, and one may send on this signal again.
            let handler = observer.handler.lock().unwrap().clone();
            if let Some(handler) = handler {
                handler(value.clone(), previous.clone());
            }
        }
    }

    pub fn last_value(&self) -> Option<T> {
        self.core.latest.lock().unwrap().clone()
    }

    /// Calls `handler` with each value sent from now on.
    pub fn observe(&self, handler: impl Fn(T) + Send + Sync + 'static) -> Subscription {
        self.add(Arc::new(move |value, _| handler(value)))
    }

    /// Calls `handler` with each value sent and the one before it.
    pub fn observe_change(
        &self,
        handler: impl Fn(T, Option<T>) + Send + Sync + 'static,
    ) -> Subscription {
        self.add(Arc::new(handler))
    }

    pub fn observer_count(&self) -> usize {
        self.core.observers.lock().unwrap().len()
    }

    pub fn remove_all_observers(&self) {
        let observers = std::mem::take(&mut *self.core.observers.lock().unwrap());
        for observer in observers {
            observer.handler.lock().unwrap().take();
        }
    }

    /// A signal that sends the last value of each burst, once `quiet` has passed with none after it.
    pub fn debounced(&self, quiet: Duration, main: &Main) -> Signal<T> {
        let derived = Signal::new();
        let out = derived.clone();
        let generation = Arc::new(AtomicU64::new(0));
        let main = main.clone();
        let subscription = self.observe(move |value| {
            let mine = generation.fetch_add(1, Ordering::SeqCst) + 1;
            let generation = generation.clone();
            let out = out.clone();
            main.spawn_work(async move {
                tokio::time::sleep(quiet).await;
                if generation.load(Ordering::SeqCst) == mine {
                    out.send(value)
                }
            });
        });
        derived.core.keep.lock().unwrap().push(subscription);
        derived
    }

    /// A signal that sends at most one value each `every`; with `latest`, the last one held back follows when
    /// the window ends.
    pub fn throttled(&self, every: Duration, latest: bool, main: &Main) -> Signal<T> {
        let derived = Signal::new();
        let out = derived.clone();
        let window: Arc<Mutex<Window<T>>> = Arc::new(Mutex::new(Window {
            fired: None,
            held: None,
            scheduled: false,
        }));
        let main = main.clone();
        let subscription = self.observe(move |value| {
            let now = std::time::Instant::now();
            let mut held = window.lock().unwrap();
            if held
                .fired
                .is_none_or(|fired| now.duration_since(fired) >= every)
            {
                held.fired = Some(now);
                drop(held);
                return out.send(value);
            }
            if !latest {
                return;
            }
            held.held = Some(value);
            if held.scheduled {
                return;
            }
            held.scheduled = true;
            let wait = every.saturating_sub(now.duration_since(held.fired.unwrap_or(now)));
            drop(held);
            let window = window.clone();
            let out = out.clone();
            main.spawn_work(async move {
                tokio::time::sleep(wait).await;
                let value = {
                    let mut held = window.lock().unwrap();
                    held.scheduled = false;
                    held.fired = Some(std::time::Instant::now());
                    held.held.take()
                };
                if let Some(value) = value {
                    out.send(value)
                }
            });
        });
        derived.core.keep.lock().unwrap().push(subscription);
        derived
    }

    fn add(&self, handler: Handler<T>) -> Subscription {
        let id = self.core.next.fetch_add(1, Ordering::SeqCst);
        let observer = Arc::new(Observer {
            id,
            handler: Mutex::new(Some(handler)),
        });
        self.core.observers.lock().unwrap().push(observer.clone());
        let core = Arc::downgrade(&self.core);
        Subscription::new(move || {
            observer.handler.lock().unwrap().take();
            if let Some(core) = core.upgrade() {
                core.observers.lock().unwrap().retain(|kept| kept.id != id);
            }
        })
    }
}
