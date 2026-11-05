//! An agent that ships as a package of its own: a word, what it does, an input that deserializes, an answer
//! that serializes, the compute it makes for itself, and a run that streams. Swift Granite's
//! `GraniteAgentPackage`.
//!
//! PIN: COMPUTE IS MADE ON THE FIRST CALL, never when the process starts: a package that holds a GPU device or
//! a model costs nothing until it is asked for, and `--describe` makes none.
//! PIN: A FIELD NAMED `directory` IS THE HOST'S: a host fills it with the folder it works in and does not show it
//! to a model. What else the host offers (the ask, notes, its model) the run reads through `context.host`.

mod any;
mod host;
mod run;

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;

pub use any::AnyAgentPackage;
pub use host::PackageHost;
pub use run::{PackageCall, PackageRun, PackageState};

use serde::de::DeserializeOwned;

use crate::{
    AgentAnswer, AgentContext, AgentHost, AgentMake, AgentRefusal, BoxError, BoxFuture,
    GraniteSchema,
};

/// A call as a roster keeps it: JSON in, JSON out, wherever the package lives.
pub type PerformFn =
    Arc<dyn Fn(Vec<u8>, AgentHost) -> BoxFuture<'static, Result<Vec<u8>, BoxError>> + Send + Sync>;

/// An agent that ships as a package.
pub trait AgentPackage: Default + Send + Sync + 'static {
    /// What a call carries, decoded from the caller's JSON. Its fields are the package's schema.
    type Input: DeserializeOwned + Send + 'static;
    /// What a run streams while it works.
    type Event: Clone + Send + 'static;
    /// What a call answers: a `granite-answer/1` answer.
    type Output: AgentAnswer;
    /// What every run computes with: made once, by `compute()`, on the first call.
    type Compute: Clone + Send + Sync + 'static;

    /// The word a host calls it by: lowercase, one word (`^[a-z][a-z0-9-]*$`), unique among what a host runs.
    const WORD: &'static str;
    /// What it does, in a sentence a host shows a person or a model.
    const SUMMARY: &'static str;

    /// A line per input field, said beside the field. Default: none.
    fn fields() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// How to use it well, in its own words: what a host puts in its prompt when the agent is asked for by name.
    fn guide() -> String {
        String::new()
    }

    /// How many calls run at once; none: no bound. Default: two.
    fn lanes() -> Option<u32> {
        Some(2)
    }

    /// What it makes, one kind each, and where.
    fn makes() -> Vec<AgentMake> {
        Vec::new()
    }

    /// The module a host names it by beside its word (`$ComicAgent`); its crate's name by default.
    fn module() -> Option<String> {
        None
    }

    /// What its input takes. Default: read off the input's own deserialization.
    fn schema() -> GraniteSchema {
        GraniteSchema::of::<Self::Input>()
    }

    /// The compute every run of this process shares, made on the first call.
    fn compute() -> impl Future<Output = Result<Self::Compute, BoxError>> + Send;

    /// The answer, read off the run's events.
    fn output(events: Vec<Self::Event>) -> Option<Self::Output>;

    /// The run: streams events with `context.deliver`, touches no state. A refusal is returned as an
    /// [`AgentRefusal`], or an error said as one ([`crate::AgentRefusing::refused`]).
    fn run(
        &self,
        input: Self::Input,
        context: AgentContext<Self::Compute, Self::Event>,
    ) -> impl Future<Output = Result<(), BoxError>> + Send;
}

/// What a package call can say besides its output.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AgentPackageError {
    /// The call's JSON did not decode as the package's input.
    Input { word: String, reason: String },
    /// The run ended without an event that answers.
    NoOutput { word: String },
    /// The call was refused, with its kind and reasons; `said` is the sentence the agent said it in.
    Refused {
        word: String,
        refusal: AgentRefusal,
        said: String,
    },
    /// The call was cancelled.
    Cancelled,
    /// The run failed by its own fault, said as it is.
    Fault(String),
}

impl fmt::Display for AgentPackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentPackageError::Input { word, reason } => {
                write!(f, "{word} could not read its input: {reason}")
            }
            AgentPackageError::NoOutput { word } => write!(f, "{word} finished without an answer"),
            AgentPackageError::Refused { said, .. } => f.write_str(said),
            AgentPackageError::Cancelled => f.write_str("cancelled"),
            AgentPackageError::Fault(said) => f.write_str(said),
        }
    }
}

impl std::error::Error for AgentPackageError {}

/// A type's module (its crate, or what it says) and its name as `Module.Type`.
pub(crate) fn names<T: ?Sized>(module: Option<String>) -> (String, String) {
    let full = std::any::type_name::<T>();
    let bare = full.split('<').next().unwrap_or(full);
    let crate_name = bare.split("::").next().unwrap_or(bare).to_owned();
    let short = bare.rsplit("::").next().unwrap_or(bare);
    let module = module.unwrap_or(crate_name);
    let r#type = format!("{module}.{short}");
    (module, r#type)
}
