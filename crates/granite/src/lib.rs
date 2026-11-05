//! Granite's kernel, in Rust.
//!
//! A Swift Granite app has one writer per subsystem: a service's center holds its state, reducers fold
//! events into it on main, and work runs in effects that send again. Agents are the streaming half: a
//! run computes off main and streams events, and its agent's own `reduce` folds each one into the
//! center's state on main, keyed by run. This crate is that kernel without SwiftUI:
//!
//! - [`Main`]: the single writer's thread (Swift's `MainActor`), and the runtime effects and runs use.
//! - [`Center`]: a state and its one writer; [`Event`] + [`GraniteReducer`] + [`Effect`]: what a send does.
//! - [`GraniteService`] + [`Relay`]: one live service per type, read and observed.
//! - [`Signal`], [`Subscription`], [`Scope`]: fan-out and what ends it.
//! - [`scaffolding`]: [`Tickets`], [`Lane`], [`RunRegistry`], [`TransientRegistry`].
//! - [`agent`]: [`GraniteAgent`], [`Agent`] (Swift's `@Agent`), [`AgentContext`], [`AgentHost`].
//!
//! PIN: ONE WRITER. A center's state is written only on main ([`Center::commit`]), and only by its
//! reducers and its agents' drains. Work never touches state: it sends, and the send commits.

pub mod agent;
mod main_queue;
mod persist;
mod reducer;
pub mod scaffolding;
mod scope;
mod service;
mod signal;
mod state;
pub mod testing;

use std::future::Future;
use std::pin::Pin;

pub use agent::{
    Agent, AgentConcurrency, AgentContext, AgentHost, AgentHostError, AgentHostModel,
    AgentLifecycle, AgentOutcome, AgentRunError, AgentRunId, AgentStart, CancelFlag, ChildFrame,
    ChildRun, GraniteAgent, RunScope, Tap,
};
pub use main_queue::Main;
pub use persist::{FilePersistence, Persistence};
pub use reducer::{Delivery, Effect, Event, GraniteReducer};
pub use scaffolding::{Lane, RunRegistry, Ticket, TicketError, Tickets, TransientRegistry};
pub use scope::{Cancellable, Scope, Subscription};
pub use service::{GraniteService, Relay};
pub use signal::Signal;
pub use state::{Center, GraniteState, Lifecycle};

/// An error a run or an effect ends with: anything that says what went wrong.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
/// A future a kernel type holds or hands out.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
