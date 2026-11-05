//! Two ends of a channel in one process, for tests of a runner or a host: what crosses is what a pipe would
//! carry, each message written as its line and read back.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{AgentChannel, ChannelError};
use crate::{AgentWire, BoxFuture};

pub struct MemoryChannel {
    tx: Mutex<Option<mpsc::UnboundedSender<AgentWire>>>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<AgentWire>>,
    sent: Mutex<Vec<AgentWire>>,
}

impl MemoryChannel {
    /// Two connected ends: what one sends, the other receives.
    pub fn pair() -> (Arc<MemoryChannel>, Arc<MemoryChannel>) {
        let (a_tx, a_rx) = mpsc::unbounded_channel();
        let (b_tx, b_rx) = mpsc::unbounded_channel();
        let a = MemoryChannel {
            tx: Mutex::new(Some(b_tx)),
            rx: tokio::sync::Mutex::new(a_rx),
            sent: Mutex::new(Vec::new()),
        };
        let b = MemoryChannel {
            tx: Mutex::new(Some(a_tx)),
            rx: tokio::sync::Mutex::new(b_rx),
            sent: Mutex::new(Vec::new()),
        };
        (Arc::new(a), Arc::new(b))
    }

    /// Everything this end sent, in order.
    pub fn sent(&self) -> Vec<AgentWire> {
        self.sent.lock().unwrap().clone()
    }
}

impl AgentChannel for MemoryChannel {
    fn send(&self, message: &AgentWire) -> Result<(), ChannelError> {
        let line = message
            .line()
            .map_err(|error| ChannelError::Encode(error.to_string()))?;
        let crossed = AgentWire::read(&line[..line.len() - 1])
            .map_err(|error| ChannelError::Encode(error.to_string()))?;
        self.sent.lock().unwrap().push(crossed.clone());
        let tx = self.tx.lock().unwrap();
        let Some(tx) = tx.as_ref() else {
            return Err(ChannelError::Closed);
        };
        tx.send(crossed).map_err(|_| ChannelError::Closed)
    }

    fn recv(&self) -> BoxFuture<'_, Option<AgentWire>> {
        Box::pin(async move { self.rx.lock().await.recv().await })
    }

    fn close(&self) {
        self.tx.lock().unwrap().take();
    }
}
