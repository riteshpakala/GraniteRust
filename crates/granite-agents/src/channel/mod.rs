//! How messages travel: whole, in order, with an end. A pipe pair today; a socket to a runner elsewhere is
//! another channel, and nothing above it changes.

mod memory;
mod pipe;

use std::fmt;

pub use memory::MemoryChannel;
pub use pipe::PipeChannel;

use crate::{AgentWire, BoxFuture};

pub trait AgentChannel: Send + Sync + 'static {
    /// Sends one message, whole, before any sent after it.
    fn send(&self, message: &AgentWire) -> Result<(), ChannelError>;
    /// The next message, in order; none once the other side closed.
    fn recv(&self) -> BoxFuture<'_, Option<AgentWire>>;
    /// Sends nothing more: the other side reads its end.
    fn close(&self);
}

#[derive(Debug)]
pub enum ChannelError {
    Closed,
    Io(std::io::Error),
    Encode(String),
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelError::Closed => f.write_str("the channel is closed"),
            ChannelError::Io(error) => write!(f, "{error}"),
            ChannelError::Encode(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ChannelError {}

/// `GRANITE_AGENT_TRACE=1` says every line sent and read on stderr, with the process's pid.
pub(crate) fn trace(arrow: &str, line: &[u8]) {
    static TRACING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*TRACING.get_or_init(|| std::env::var("GRANITE_AGENT_TRACE").as_deref() == Ok("1")) {
        return;
    }
    let shown = String::from_utf8_lossy(&line[..line.len().min(300)]);
    eprintln!(
        "[granite-agent {}] {arrow} {}",
        std::process::id(),
        shown.trim_end_matches('\n')
    );
}
