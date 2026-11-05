//! A channel over two file descriptors: lines of JSON written to one, read from the other.
//!
//! PIN: READ WHAT HAS ARRIVED. `read(2)` returns what is in the pipe; a line of a live call is never held until
//! the process exits.
//! PIN: READ ON A THREAD OF ITS OWN, never a runtime worker: a host may hold many channels for the life of the
//! process.
//! PIN: A LINE THAT IS NOT A MESSAGE IS DROPPED, and said on `unreadable`: it is a fault to report, not a reason
//! to stop reading.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{AgentChannel, ChannelError, trace};
use crate::{AgentWire, BoxFuture};

pub struct PipeChannel {
    messages: tokio::sync::Mutex<mpsc::UnboundedReceiver<AgentWire>>,
    writer: Mutex<Option<File>>,
}

impl PipeChannel {
    /// Reads `reading` from now on, writes `writing`. Both are closed when the channel is done with them.
    pub fn new(
        reading: OwnedFd,
        writing: OwnedFd,
        unreadable: impl Fn(&str) + Send + Sync + 'static,
    ) -> Arc<PipeChannel> {
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("granite.agent.channel".into())
            .spawn(move || {
                let mut reading = File::from(reading);
                let mut buffer: Vec<u8> = Vec::new();
                let mut chunk = vec![0u8; 1 << 16];
                loop {
                    let count = match reading.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(count) => count,
                        Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    };
                    buffer.extend_from_slice(&chunk[..count]);
                    while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                        let line: Vec<u8> = buffer.drain(..=end).collect();
                        let line = &line[..line.len() - 1];
                        if line.iter().all(|byte| *byte == b' ' || *byte == b'\r') {
                            continue;
                        }
                        trace("read", line);
                        match AgentWire::read(line) {
                            Ok(message) => {
                                if tx.send(message).is_err() {
                                    return;
                                }
                            }
                            Err(_) => {
                                unreadable(&String::from_utf8_lossy(&line[..line.len().min(200)]))
                            }
                        }
                    }
                }
            })
            .expect("granite: the channel's reader could not start");
        Arc::new(PipeChannel {
            messages: tokio::sync::Mutex::new(rx),
            writer: Mutex::new(Some(File::from(writing))),
        })
    }
}

impl AgentChannel for PipeChannel {
    fn send(&self, message: &AgentWire) -> Result<(), ChannelError> {
        let line = message
            .line()
            .map_err(|error| ChannelError::Encode(error.to_string()))?;
        trace("sent", &line);
        let mut writer = self.writer.lock().unwrap();
        let Some(file) = writer.as_mut() else {
            return Err(ChannelError::Closed);
        };
        file.write_all(&line).map_err(ChannelError::Io)
    }

    fn recv(&self) -> BoxFuture<'_, Option<AgentWire>> {
        Box::pin(async move { self.messages.lock().await.recv().await })
    }

    fn close(&self) {
        self.writer.lock().unwrap().take();
    }
}
