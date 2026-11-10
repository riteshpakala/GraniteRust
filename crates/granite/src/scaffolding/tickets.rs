//! A typed reply slot for a ticketed dispatch: Swift's `GraniteTickets`.
//!
//! PIN: OPEN BEFORE SEND. A resolve that lands before `wait` is kept in the slot, so a fast reducer never
//! resolves into nothing. Dropping the waiting future cancels the ticket; a late resolve is dropped.
//! PIN: A SETTLE WAITS FOR MAIN. Resolve and fail let main run every turn sent before them, so a write an
//! effect sent ahead of its resolve is in the state when the waiter wakes and reads it.
//! PIN: A SETTLE HANDS OFF AFTER THE UNLOCK. The waiter is answered, and a value nobody will read is
//! dropped, once the slots' lock is released.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::oneshot;

use crate::Main;
use crate::locked::Locked;

/// One reply slot, by id: what a reducer's payload carries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Ticket {
    pub id: u64,
}

/// What a ticket can say besides its value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TicketError {
    /// A second `wait` on a ticket someone is already waiting on.
    AlreadyAwaited(u64),
    /// The callee refused the request: busy, not ready, not open.
    Refused(String),
    /// The ticket was cancelled, or is unknown.
    Cancelled,
    /// The work behind it failed.
    Failed(String),
}

impl std::fmt::Display for TicketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TicketError::AlreadyAwaited(id) => write!(f, "ticket {id} is already awaited"),
            TicketError::Refused(why) | TicketError::Failed(why) => f.write_str(why),
            TicketError::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for TicketError {}

enum Slot<V> {
    Open,
    Waiting(oneshot::Sender<Result<V, TicketError>>),
    Settled(Result<V, TicketError>),
}

/// Reply slots for values of `V`. Clones share slots.
pub struct Tickets<V: Send + 'static> {
    main: Main,
    slots: Arc<Locked<HashMap<u64, Slot<V>>>>,
    next: Arc<AtomicU64>,
}

impl<V: Send + 'static> Clone for Tickets<V> {
    fn clone(&self) -> Self {
        Tickets {
            main: self.main.clone(),
            slots: self.slots.clone(),
            next: self.next.clone(),
        }
    }
}

struct WaitGuard<V: Send + 'static> {
    tickets: Tickets<V>,
    id: u64,
    armed: bool,
}

impl<V: Send + 'static> Drop for WaitGuard<V> {
    fn drop(&mut self) {
        if self.armed {
            self.tickets.cancel(self.id)
        }
    }
}

impl<V: Send + 'static> Tickets<V> {
    pub fn new(main: &Main) -> Self {
        Tickets {
            main: main.clone(),
            slots: Default::default(),
            next: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Tickets on [`Main::global`].
    pub fn global() -> Self {
        Self::new(Main::global())
    }

    /// A new slot, open: call before sending the payload that carries its id.
    pub fn open(&self) -> Ticket {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let replaced = self.slots.with(|slots| slots.insert(id, Slot::Open));
        drop(replaced);
        Ticket { id }
    }

    /// Suspends until the ticket is resolved or failed.
    pub async fn wait(&self, id: u64) -> Result<V, TicketError> {
        let receiver = self.slots.with(|slots| match slots.remove(&id) {
            Some(Slot::Settled(result)) => Err(result),
            Some(Slot::Open) => {
                let (tx, rx) = oneshot::channel();
                slots.insert(id, Slot::Waiting(tx));
                Ok(rx)
            }
            Some(waiting @ Slot::Waiting(_)) => {
                slots.insert(id, waiting);
                Err(Err(TicketError::AlreadyAwaited(id)))
            }
            None => Err(Err(TicketError::Cancelled)),
        });
        let receiver = match receiver {
            Ok(receiver) => receiver,
            Err(answered) => return answered,
        };
        let mut guard = WaitGuard {
            tickets: self.clone(),
            id,
            armed: true,
        };
        let result = receiver.await.unwrap_or(Err(TicketError::Cancelled));
        guard.armed = false;
        result
    }

    /// Answers the waiter, or keeps the value for one that has not waited yet, after main has run what was
    /// sent to it before this call.
    pub async fn resolve(&self, id: u64, value: V) {
        self.main.turn().await;
        self.settle(id, Ok(value))
    }

    /// Fails the waiter with `error`, after main has run what was sent to it before this call.
    pub async fn fail(&self, id: u64, error: TicketError) {
        self.main.turn().await;
        self.settle(id, Err(error))
    }

    /// Fails the waiter as cancelled; a late resolve is then dropped.
    pub fn cancel(&self, id: u64) {
        self.settle(id, Err(TicketError::Cancelled))
    }

    /// Tickets nobody has settled and awaited yet.
    pub fn pending(&self) -> usize {
        self.slots.with(|slots| slots.len())
    }

    fn settle(&self, id: u64, result: Result<V, TicketError>) {
        // Whoever waits, or the result nobody will read (settled already, or cancelled), comes out.
        let waiter = self.slots.with(|slots| match slots.remove(&id) {
            Some(Slot::Waiting(tx)) => Ok((tx, result)),
            Some(Slot::Open) => {
                slots.insert(id, Slot::Settled(result));
                Err(None)
            }
            Some(settled @ Slot::Settled(_)) => {
                slots.insert(id, settled);
                Err(Some(result))
            }
            None => Err(Some(result)),
        });
        match waiter {
            Ok((tx, result)) => {
                let _ = tx.send(result);
            }
            Err(unread) => drop(unread),
        }
    }
}
