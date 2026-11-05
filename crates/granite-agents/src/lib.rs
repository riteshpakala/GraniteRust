//! The Granite agent standard, `granite-agent/1`, in Rust.
//!
//! An agent package is a crate that declares an [`AgentPackage`]: a word, what it does, an input that
//! deserializes, an answer that serializes, the compute it makes for itself, and a run that streams
//! events. A runner is a binary whose `main` hands its packages to [`AgentServer::main`]: with
//! `--describe` it says what it carries, otherwise it answers one call on stdin and stdout and exits. A
//! host calls a runner with [`AgentProcess`], offers installed runners from an [`AgentShelf`], and
//! installs a package's repository with [`AgentInstaller`] (`cargo build`). The wire, the entry, the
//! installation and the answer are byte-compatible with Swift Granite's, so a Swift host runs a Rust
//! runner and a Rust host runs a Swift one. The standard is `Schema/GraniteAgent.md` at the repo root.
//!
//! ```ignore
//! // src/main.rs of an agent package: the whole runner.
//! fn main() {
//!     granite_agents::AgentServer::main(granite_agents::roster![my_agent::MyAgent])
//! }
//! ```

pub mod answer;
pub mod channel;
mod entry;
pub mod host_ext;
pub mod install;
pub mod json;
mod make;
pub mod package;
pub mod process;
mod refusal;
mod remote;
mod roster;
pub mod schema;
pub mod server;
mod shelf;
mod wire;
mod worker;

pub use answer::{
    AgentAnswer, AgentExpected, AgentMade, AgentPayload, AgentProblem, Answer, AnswerCheck,
    AnswerStandard, MadeState,
};
pub use channel::{AgentChannel, ChannelError, MemoryChannel, PipeChannel};
pub use entry::{Entry, EntryKind};
pub use install::{AgentInstallError, AgentInstallation, AgentInstaller, AgentSource, Progress};
pub use make::AgentMake;
pub use package::{
    AgentPackage, AgentPackageError, AnyAgentPackage, PackageCall, PackageHost, PackageRun,
    PackageState,
};
pub use process::{AgentProcess, AgentProcessCost, AgentRemoteError};
pub use refusal::{AgentRefusal, AgentRefusing, RefusalKind, SaidRefusal};
pub use remote::RemoteAgent;
pub use roster::{Roster, RosterEntry};
pub use schema::{Field, GraniteSchema};
pub use server::{AgentServer, Exit};
pub use shelf::AgentShelf;
pub use wire::{AgentWire, AskKind, Failure, Hello, ModelAnswer, ModelAsk, Perform, WireError};
pub use worker::{AnyWorkerSpec, GraniteWorkerSpec, WorkerBudget};

pub use granite::{AgentContext, AgentHost, AgentHostError, AgentHostModel, BoxError, BoxFuture};

/// The roster a runner carries: `roster![A, B]` is `Roster::empty().with_package::<A>().with_package::<B>()`.
#[macro_export]
macro_rules! roster {
    ($($package:ty),* $(,)?) => {
        $crate::Roster::empty()$(.with_package::<$package>())*
    };
}
