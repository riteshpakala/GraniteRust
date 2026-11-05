//! A state and its one writer: a Swift Granite center, its store and its lifecycle.

use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::Main;
use crate::persist::Persistence;

/// What a center holds: a value, compared to know whether a commit changed anything.
pub trait GraniteState: Clone + PartialEq + Default + Send + Sync + 'static {}
impl<T: Clone + PartialEq + Default + Send + Sync + 'static> GraniteState for T {}

/// When a hook fires: as a host shows the center, hides it, or starts its tasks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lifecycle {
    OnAppear,
    OnDisappear,
    OnTask,
    None,
}

type Hook = Arc<dyn Fn() + Send + Sync>;

/// A state and its writer. Clones share one state.
///
/// PIN: COMMITS ARE MAIN'S. [`Center::commit`] and [`Center::update`] run on main; anything else sends.
pub struct Center<S: GraniteState> {
    inner: Arc<Inner<S>>,
}

struct Inner<S> {
    main: Main,
    label: String,
    state: watch::Sender<S>,
    hooks: Mutex<Vec<(Lifecycle, Hook)>>,
    store: Option<Arc<dyn Persistence<S>>>,
}

impl<S: GraniteState> Clone for Center<S> {
    fn clone(&self) -> Self {
        Center {
            inner: self.inner.clone(),
        }
    }
}

impl<S: GraniteState> Center<S> {
    /// A center at its state's default.
    pub fn new(main: &Main) -> Self {
        Self::with_state(main, S::default())
    }

    /// A center that starts at `state`.
    pub fn with_state(main: &Main, state: S) -> Self {
        Self::make(main, std::any::type_name::<S>().to_owned(), state, None)
    }

    /// A center whose state is read from `store` (its default when there is none) and written back on
    /// every commit that changes it.
    pub fn persisted(main: &Main, store: Arc<dyn Persistence<S>>) -> Self {
        let state = store.load().unwrap_or_default();
        Self::make(
            main,
            std::any::type_name::<S>().to_owned(),
            state,
            Some(store),
        )
    }

    fn make(main: &Main, label: String, state: S, store: Option<Arc<dyn Persistence<S>>>) -> Self {
        let (state, _) = watch::channel(state);
        Center {
            inner: Arc::new(Inner {
                main: main.clone(),
                label,
                state,
                hooks: Mutex::new(Vec::new()),
                store,
            }),
        }
    }

    /// The same center, named for logs and run ids. Name it before sharing it.
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("granite: label a center before sharing it")
            .label = label.into();
        self
    }

    pub fn label(&self) -> &str {
        &self.inner.label
    }

    pub fn main(&self) -> &Main {
        &self.inner.main
    }

    /// The state now, copied.
    pub fn state(&self) -> S {
        self.inner.state.borrow().clone()
    }

    /// Reads the state now without copying it.
    pub fn with<T>(&self, read: impl FnOnce(&S) -> T) -> T {
        read(&self.inner.state.borrow())
    }

    /// A receiver that wakes on every commit that changed the state.
    pub fn watch(&self) -> watch::Receiver<S> {
        self.inner.state.subscribe()
    }

    /// Main-only: writes `state` when it differs from what is held, and saves it when the center persists.
    pub fn commit(&self, state: S) {
        debug_assert!(
            self.inner.main.is_main(),
            "granite: {} committed off main",
            self.inner.label
        );
        let mut saved = None;
        self.inner.state.send_if_modified(|held| {
            if *held == state {
                return false;
            }
            *held = state;
            if self.inner.store.is_some() {
                saved = Some(held.clone());
            }
            true
        });
        if let (Some(store), Some(state)) = (&self.inner.store, saved) {
            store.save(&state);
        }
    }

    /// Main-only: the state changed by `change`, committed.
    pub fn update(&self, change: impl FnOnce(&mut S)) {
        let mut state = self.state();
        change(&mut state);
        self.commit(state);
    }

    /// Calls `hook` on main each time the center reaches `when`.
    pub fn on_lifecycle(&self, when: Lifecycle, hook: impl Fn() + Send + Sync + 'static) {
        self.inner
            .hooks
            .lock()
            .unwrap()
            .push((when, Arc::new(hook)));
    }

    /// The host shows the center: its `OnAppear` hooks fire on main.
    pub fn appear(&self) {
        self.fire(Lifecycle::OnAppear);
    }

    /// The host hides the center: its `OnDisappear` hooks fire on main.
    pub fn disappear(&self) {
        self.fire(Lifecycle::OnDisappear);
    }

    /// The host starts the center's tasks: its `OnTask` hooks fire on main.
    pub fn perform_tasks(&self) {
        self.fire(Lifecycle::OnTask);
    }

    fn fire(&self, when: Lifecycle) {
        let hooks: Vec<Hook> = self
            .inner
            .hooks
            .lock()
            .unwrap()
            .iter()
            .filter(|(at, _)| *at == when)
            .map(|(_, hook)| hook.clone())
            .collect();
        self.inner.main.run(move || {
            for hook in hooks {
                hook()
            }
        });
    }
}
