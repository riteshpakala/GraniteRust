//! The pieces every service shares: a ticketed reply, a serial lane, cancellable work by id, residents by key.

mod lane;
mod registry;
mod tickets;

pub use lane::Lane;
pub use registry::{RunRegistry, TransientRegistry};
pub use tickets::{Ticket, TicketError, Tickets};
