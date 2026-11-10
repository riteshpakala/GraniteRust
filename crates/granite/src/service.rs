//! A subsystem's one live instance: Swift's `GraniteService`, `@Relay` and `Live.service`.
//!
//! PIN: ONE PER TYPE PER PROCESS. [`Relay::live`] makes the service once, outside the registry's lock (a
//! service whose making reaches another relay cannot deadlock), and every later relay shares it. One
//! made by a caller that lost a race is dropped after the unlock. A reducer reaches its own service with
//! `Relay::<Self>::live()` from its effect, never by storing one.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::locked::Locked;
use crate::{Center, GraniteState, Main};

/// A subsystem: a center, its events, its agents.
pub trait GraniteService: Send + Sync + Sized + 'static {
    type State: GraniteState;
    /// Builds the service's center and events on `main`.
    fn make(main: &Main) -> Self;
    fn center(&self) -> &Center<Self::State>;
}

/// A handle to a service's live instance.
pub struct Relay<S: GraniteService>(Arc<S>);

impl<S: GraniteService> Clone for Relay<S> {
    fn clone(&self) -> Self {
        Relay(self.0.clone())
    }
}

impl<S: GraniteService> Deref for Relay<S> {
    type Target = S;
    fn deref(&self) -> &S {
        &self.0
    }
}

type Live = HashMap<TypeId, Arc<dyn Any + Send + Sync>>;

fn registry() -> &'static Locked<Live> {
    static LIVE: OnceLock<Locked<Live>> = OnceLock::new();
    LIVE.get_or_init(Default::default)
}

impl<S: GraniteService> Relay<S> {
    /// The live `S`, made on [`Main::global`] the first time any relay asks.
    pub fn live() -> Relay<S> {
        let id = TypeId::of::<S>();
        if let Some(found) = registry().with(|live| live.get(&id).cloned()) {
            return Relay(
                found
                    .downcast::<S>()
                    .expect("granite: a relay of another type"),
            );
        }
        let made: Arc<dyn Any + Send + Sync> = Arc::new(S::make(Main::global()));
        let (kept, lost) = registry().with(|live| match live.get(&id) {
            Some(found) => (found.clone(), Some(made)),
            None => {
                live.insert(id, made.clone());
                (made, None)
            }
        });
        drop(lost);
        Relay(
            kept.downcast::<S>()
                .expect("granite: a relay of another type"),
        )
    }

    /// A relay to a service made by hand (a test's, a host's own main).
    pub fn of(service: Arc<S>) -> Relay<S> {
        Relay(service)
    }

    pub fn state(&self) -> S::State {
        self.0.center().state()
    }

    pub fn watch(&self) -> watch::Receiver<S::State> {
        self.0.center().watch()
    }

    /// Calls `handler` on main with `select(state)` whenever it changes, until the [`Observation`] is
    /// dropped. A delivery already queued when it is dropped does not run.
    pub fn observe<V, H>(
        &self,
        select: impl Fn(&S::State) -> V + Send + 'static,
        handler: H,
    ) -> Observation
    where
        V: PartialEq + Clone + Send + 'static,
        H: FnMut(V) + Send + 'static,
    {
        let center = self.0.center().clone();
        let main = center.main().clone();
        // The observation holds the handler's one strong handle; the watcher and each delivery hold it weakly.
        let handler = Arc::new(Mutex::new(handler));
        let weak = Arc::downgrade(&handler);
        let mut changes = center.watch();
        // `select` is the author's: it reads a copy, never the state under the watch's lock. The baseline
        // is taken here, so a commit that lands before the watcher first runs is still a change.
        let state = changes.borrow_and_update().clone();
        let mut last = select(&state);
        drop(state);
        let task = main.clone().spawn_work(async move {
            while changes.changed().await.is_ok() {
                let state = changes.borrow_and_update().clone();
                let now = select(&state);
                if now == last {
                    continue;
                }
                last = now.clone();
                let handler = weak.clone();
                main.enqueue(move || deliver(&handler, now));
            }
        });
        Observation {
            _handler: handler,
            task,
        }
    }
}

/// One delivery, on main: the handler runs only if the observation is still held. The handler's lock
/// only orders its calls (all on main); a handle upgraded here is dropped after that lock is released.
fn deliver<V, H: FnMut(V)>(handler: &Weak<Mutex<H>>, value: V) {
    let Some(handler) = handler.upgrade() else {
        return;
    };
    let mut call = handler.lock().unwrap();
    (*call)(value);
    drop(call);
    drop(handler);
}

/// A live [`Relay::observe`]: Combine's `AnyCancellable`. Dropping it ends the observation and lets its
/// handler go; keep it for as long as you observe.
#[must_use = "an Observation ends when it is dropped: keep it for as long as you observe"]
pub struct Observation {
    _handler: Arc<dyn Any + Send + Sync>,
    task: JoinHandle<()>,
}

impl Drop for Observation {
    /// Stops the watcher, which holds only a weak handle to the handler: the handler goes with this.
    fn drop(&mut self) {
        self.task.abort();
    }
}
