//! What ends a subscription: Swift's `GraniteCancellable` and `Prospect`.
//!
//! PIN: DROPPING ENDS IT. A [`Subscription`] dropped is cancelled, the Rust way; keep one alive by holding
//! it, storing it in a [`Scope`], or [`Subscription::forget`]. A scope's cancel is terminal: what is stored
//! after it is cancelled at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A token that runs its cancel once, from any thread.
#[derive(Clone)]
pub struct Cancellable {
    inner: Arc<CancellableInner>,
}

struct CancellableInner {
    cancelled: AtomicBool,
    on_cancel: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Cancellable {
    pub fn new(on_cancel: impl FnOnce() + Send + 'static) -> Self {
        Cancellable {
            inner: Arc::new(CancellableInner {
                cancelled: AtomicBool::new(false),
                on_cancel: Mutex::new(Some(Box::new(on_cancel))),
            }),
        }
    }

    pub fn cancel(&self) {
        if self.inner.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        let action = self.inner.on_cancel.lock().unwrap().take();
        if let Some(action) = action {
            action()
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }
}

/// A live observation: cancelled when dropped, unless forgotten or stored.
pub struct Subscription {
    token: Cancellable,
    armed: bool,
}

impl Subscription {
    pub fn new(on_cancel: impl FnOnce() + Send + 'static) -> Self {
        Subscription {
            token: Cancellable::new(on_cancel),
            armed: true,
        }
    }

    pub fn cancel(&self) {
        self.token.cancel()
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Keeps the observation for the life of the process.
    pub fn forget(mut self) {
        self.armed = false;
    }

    /// Hands the observation to `scope`, which ends it when it is cancelled or dropped.
    pub fn store_in(self, scope: &Scope) {
        scope.store(self)
    }

    /// A handle that cancels this subscription without owning it.
    pub fn token(&self) -> Cancellable {
        self.token.clone()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if self.armed {
            self.token.cancel()
        }
    }
}

/// Owns subscriptions and child scopes; cancelling it, or dropping its last handle, ends them all.
#[derive(Clone)]
pub struct Scope {
    inner: Arc<ScopeInner>,
}

struct ScopeInner {
    label: String,
    cancelled: AtomicBool,
    subscriptions: Mutex<Vec<Subscription>>,
    children: Mutex<Vec<Scope>>,
}

impl Drop for ScopeInner {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        for subscription in self.subscriptions.get_mut().unwrap().drain(..) {
            subscription.cancel();
        }
    }
}

impl Scope {
    pub fn root(label: &str) -> Scope {
        Scope {
            inner: Arc::new(ScopeInner {
                label: label.to_owned(),
                cancelled: AtomicBool::new(false),
                subscriptions: Mutex::new(Vec::new()),
                children: Mutex::new(Vec::new()),
            }),
        }
    }

    /// A scope cancelled with this one.
    pub fn child(&self, label: &str) -> Scope {
        let child = Scope::root(&format!("{}/{}", self.inner.label, label));
        if self.is_cancelled() {
            child.cancel();
        } else {
            self.inner.children.lock().unwrap().push(child.clone());
        }
        child
    }

    pub fn store(&self, subscription: Subscription) {
        if self.is_cancelled() {
            return subscription.cancel();
        }
        self.inner.subscriptions.lock().unwrap().push(subscription);
    }

    pub fn cancel(&self) {
        if self.inner.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        let subscriptions = std::mem::take(&mut *self.inner.subscriptions.lock().unwrap());
        for subscription in subscriptions {
            subscription.cancel();
        }
        // Children stay listed, cancelled, so the tree still reads after it ended.
        let children = self.inner.children.lock().unwrap().clone();
        for child in children {
            child.cancel();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub fn subscription_count(&self) -> usize {
        self.inner.subscriptions.lock().unwrap().len()
    }

    pub fn children(&self) -> usize {
        self.inner.children.lock().unwrap().len()
    }

    /// The tree, one line a scope: label, subscriptions, state.
    pub fn dump(&self) -> String {
        let mut lines = Vec::new();
        self.dump_into(0, &mut lines);
        lines.join("\n")
    }

    fn dump_into(&self, depth: usize, lines: &mut Vec<String>) {
        lines.push(format!(
            "{}{} · {} subscriptions{}",
            "  ".repeat(depth),
            self.inner.label,
            self.subscription_count(),
            if self.is_cancelled() {
                " · cancelled"
            } else {
                ""
            }
        ));
        for child in self.inner.children.lock().unwrap().iter() {
            child.dump_into(depth + 1, lines);
        }
    }
}
