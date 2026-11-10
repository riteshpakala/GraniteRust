//! Values sent to receivers that are held weakly: Swift's `GraniteEmitter`.
//!
//! PIN: NO TOKENS. [`Emitter::on`] keeps its receiver as a `Weak` and hands the live `Arc` back to the
//! handler on each send, so the handler need not capture it. The subscription ends when the receiver is
//! released: nothing is stored, cancelled, or torn down in a `Drop`. [`Emitter::off`] removes one early
//! and is never needed for correctness.
//! PIN: DELIVERY IS SYNCHRONOUS, IN ORDER, LOCK RELEASED. A send copies the list under the lock and
//! delivers on the sender's thread with it released, so a handler may send, subscribe or unsubscribe on
//! the same emitter. A receiver released by an earlier handler in the same send is not reached; `off` and
//! a replaced key take effect from the next send.
//! PIN: NOTHING IS DROPPED UNDER THE LOCK. Every change to the list hands back the entries it removed,
//! and they are dropped after the unlock: an entry owns its handler, and a handler's captures may send
//! to this very emitter as they go.
//! PIN: A HANDLER THAT CAPTURES ITS OWN RECEIVER OR ITS OWN EMITTER KEEPS IT ALIVE. Reach what it needs
//! through the receiver it is handed.

use std::sync::{Arc, Weak};

use crate::locked::Locked;

/// Names one subscription on an emitter: subscribing again under the same key replaces it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EmitterKey(String);

impl EmitterKey {
    pub fn new(name: impl Into<String>) -> Self {
        EmitterKey(name.into())
    }
}

/// One subscription's receiver and handler, behind the value type it receives.
trait Receiving<T>: Send + Sync {
    /// Whether the receiver is still alive. Never upgrades: an upgrade here could make this thread the
    /// receiver's last owner while a lock is held.
    fn is_alive(&self) -> bool;
    /// The receiver's address, for `off`. The `Weak` keeps the allocation, so no live `Arc` shares it.
    fn address(&self) -> *const ();
    /// Delivers `value` when the receiver is alive; false when it is gone.
    fn deliver(&self, value: &T) -> bool;
}

struct Held<R, F> {
    receiver: Weak<R>,
    handler: F,
}

impl<T, R, F> Receiving<T> for Held<R, F>
where
    R: Send + Sync + 'static,
    F: Fn(&Arc<R>, &T) + Send + Sync + 'static,
{
    fn is_alive(&self) -> bool {
        self.receiver.strong_count() > 0
    }

    fn address(&self) -> *const () {
        self.receiver.as_ptr().cast()
    }

    fn deliver(&self, value: &T) -> bool {
        match self.receiver.upgrade() {
            Some(receiver) => {
                (self.handler)(&receiver, value);
                true
            }
            None => false,
        }
    }
}

struct Entry<T> {
    key: Option<EmitterKey>,
    receiver: Box<dyn Receiving<T>>,
}

/// Sends values to receivers, synchronously and in subscription order. Clones share the receivers.
pub struct Emitter<T: 'static> {
    entries: Arc<Locked<Vec<Arc<Entry<T>>>>>,
}

impl<T: 'static> Clone for Emitter<T> {
    fn clone(&self) -> Self {
        Emitter {
            entries: self.entries.clone(),
        }
    }
}

impl<T: 'static> Default for Emitter<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static> Emitter<T> {
    pub fn new() -> Self {
        Emitter {
            entries: Arc::new(Locked::new(Vec::new())),
        }
    }

    /// Sends `value` to every live receiver, in subscription order, on this thread.
    pub fn send(&self, value: T) {
        let snapshot = self.entries.with(|list| list.clone());
        let mut released = false;
        for entry in &snapshot {
            if !entry.receiver.deliver(&value) {
                released = true;
            }
        }
        // The snapshot may hold the last handle to an entry a handler removed: let it go unlocked.
        drop(snapshot);
        if released {
            self.compact();
        }
    }

    /// Subscribes `receiver`, held weakly, until it is released. With a `key`, the subscription already
    /// under it is removed first and this one is added last, whoever its receiver was.
    pub fn on<R, F>(&self, receiver: &Arc<R>, key: Option<EmitterKey>, handler: F)
    where
        R: Send + Sync + 'static,
        F: Fn(&Arc<R>, &T) + Send + Sync + 'static,
    {
        let entry = Arc::new(Entry {
            key: key.clone(),
            receiver: Box::new(Held {
                receiver: Arc::downgrade(receiver),
                handler,
            }),
        });
        let removed = self.rewrite(Some(entry), |existing| key.is_some() && existing.key == key);
        drop(removed);
    }

    /// Removes `receiver`'s subscriptions: all of them, or only the one under `key`.
    pub fn off<R>(&self, receiver: &Arc<R>, key: Option<&EmitterKey>) {
        let address: *const () = Arc::as_ptr(receiver).cast();
        let removed = self.rewrite(None, |existing| {
            std::ptr::addr_eq(existing.receiver.address(), address)
                && key.is_none_or(|key| existing.key.as_ref() == Some(key))
        });
        drop(removed);
    }

    /// How many subscriptions still have a live receiver.
    pub fn receiver_count(&self) -> usize {
        self.entries.with(|list| {
            list.iter()
                .filter(|entry| entry.receiver.is_alive())
                .count()
        })
    }

    /// Removes every subscription: an owner taking the emitter over.
    pub fn remove_all(&self) {
        let removed = self.entries.with(std::mem::take);
        drop(removed);
    }

    /// Drops the subscriptions whose receivers are gone.
    fn compact(&self) {
        let removed = self.rewrite(None, |_| false);
        drop(removed);
    }

    /// Rebuilds the list under the lock: drops the entries whose receiver is gone and those `remove`
    /// picks, then appends `entry`. Returns what it removed, to be dropped after the unlock. `remove`
    /// runs under the lock and must not call out.
    fn rewrite(
        &self,
        entry: Option<Arc<Entry<T>>>,
        remove: impl Fn(&Entry<T>) -> bool,
    ) -> Vec<Arc<Entry<T>>> {
        self.entries.with(|list| {
            let (removed, mut kept): (Vec<_>, Vec<_>) = std::mem::take(list)
                .into_iter()
                .partition(|existing| !existing.receiver.is_alive() || remove(existing));
            kept.extend(entry);
            *list = kept;
            removed
        })
    }
}
