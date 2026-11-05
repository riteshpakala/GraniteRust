//! The agent's side of `granite-agent/1`: the whole `main` of a process that carries agent packages. With
//! `--describe` it says what it carries and exits; otherwise it answers one call on stdin and stdout, offering
//! the run the host's folder, ask, notes and model over the same pipe, and exits.
//!
//! PIN: STDOUT IS THE WIRE'S ALONE. The real stdout is moved aside before anything runs and the descriptor the
//! process calls stdout is pointed at stderr, so a package's `println!` can never put a line on the wire.

mod call;
mod model;

use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;

use granite::Main;

use crate::{AgentChannel, AgentWire, PipeChannel, Roster};

pub(crate) use call::ServerCall;

/// How the process ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    /// The call was answered with its output.
    Answered = 0,
    /// The call failed, and said why.
    Failed = 1,
    /// The host said something that is not a call, or left before making one.
    Unreadable = 2,
    /// The call was refused, with its kind and reasons.
    Refused = 3,
    /// The host cancelled the call, or asked the process to end (SIGTERM).
    Cancelled = 130,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }
}

pub struct AgentServer;

impl AgentServer {
    /// A runner's `main`: never returns.
    pub fn main(roster: Roster) -> ! {
        Self::main_with(roster, std::env::args().collect())
    }

    pub fn main_with(roster: Roster, arguments: Vec<String>) -> ! {
        // SAFETY: descriptors 1 and 2 are the process's own; nothing has written to stdout yet.
        let wire = unsafe {
            let wire = libc::dup(libc::STDOUT_FILENO);
            libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO);
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            OwnedFd::from_raw_fd(wire)
        };
        // SAFETY: descriptor 0 is the process's stdin, owned by the channel from here on.
        let stdin = unsafe { OwnedFd::from_raw_fd(libc::STDIN_FILENO) };
        let channel = PipeChannel::new(stdin, wire, |line| {
            eprintln!("granite-agent: an unreadable line from the host: {line}")
        });
        let main = Main::global();
        let exit = main.block_on(async move {
            if arguments.iter().any(|argument| argument == "--describe") {
                let _ = channel.send(&AgentWire::Entries(roster.describe()));
                channel.close();
                return Exit::Answered;
            }
            let call = ServerCall::new(channel.clone() as Arc<dyn AgentChannel>, roster);
            let cancelling = call.clone();
            tokio::spawn(async move {
                if let Ok(mut terminate) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                {
                    terminate.recv().await;
                    cancelling.cancel();
                }
            });
            let exit = call.serve().await;
            channel.close();
            exit
        });
        std::process::exit(exit.code())
    }

    /// One call on `channel`, answered from `roster`: what `main` runs, without the process around it.
    pub async fn serve(channel: Arc<dyn AgentChannel>, roster: Roster) -> Exit {
        ServerCall::new(channel, roster).serve().await
    }
}
