//! What the app that runs a package offers the run beside its input: its folder, what the person asked,
//! a line of progress it shows, and the model it has loaded.
//!
//! PIN: OFFERED, NEVER ASSUMED. [`AgentHost::none`] offers nothing, so an app without a model still runs
//! every package; one that cannot work without the model asks with [`AgentHost::model_for`].
//! PIN: THE ASK IS NOT A PARAMETER. A model writes a call's input; it never writes what the person asked.

use std::fmt;
use std::sync::Arc;

use crate::BoxFuture;

/// The model a host has loaded, as a package may ask it: a JSON object that fits a JSON Schema, or text.
pub trait AgentHostModel: Send + Sync + 'static {
    /// A JSON object that fits `schema` (JSON Schema bytes); `NoAnswer` when nothing parsed or `seconds` passed.
    fn object(
        &self,
        schema: Vec<u8>,
        system: String,
        user: String,
        max_tokens: u32,
        seconds: f64,
    ) -> BoxFuture<'_, Result<Vec<u8>, AgentHostError>>;
    /// Text, whole.
    fn text(
        &self,
        system: String,
        user: String,
        max_tokens: u32,
        seconds: f64,
    ) -> BoxFuture<'_, Result<String, AgentHostError>>;
}

/// What asking the host can say besides an answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AgentHostError {
    /// The package asked for the host's model, and the host has none.
    NoModel { word: String },
    /// The model said nothing that parses, or its time was up.
    NoAnswer(String),
    /// The model's JSON did not decode as what was asked.
    Answer(String),
}

impl fmt::Display for AgentHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentHostError::NoModel { word } => write!(
                f,
                "{word} asks the host's model, and this host has none loaded"
            ),
            AgentHostError::NoAnswer(reason) => {
                write!(f, "the host's model gave no answer: {reason}")
            }
            AgentHostError::Answer(reason) => write!(
                f,
                "the host's model answered something that does not decode: {reason}"
            ),
        }
    }
}

impl std::error::Error for AgentHostError {}

/// What a host offers a run beside its input.
#[derive(Clone)]
pub struct AgentHost {
    /// The folder the host works in; none when it has none.
    pub directory: Option<String>,
    /// What the person asked, in their words; none when no person asked (a script, a test).
    pub ask: Option<String>,
    /// The model the host has loaded; none when it has none.
    pub model: Option<Arc<dyn AgentHostModel>>,
    note: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Default for AgentHost {
    fn default() -> Self {
        Self::none()
    }
}

impl fmt::Debug for AgentHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentHost")
            .field("directory", &self.directory)
            .field("ask", &self.ask)
            .field("model", &self.model.is_some())
            .finish()
    }
}

impl AgentHost {
    /// A host that offers nothing: a call made with JSON alone.
    pub fn none() -> Self {
        AgentHost {
            directory: None,
            ask: None,
            model: None,
            note: Arc::new(|_| {}),
        }
    }

    pub fn with_directory(mut self, directory: impl Into<String>) -> Self {
        self.directory = Some(directory.into());
        self
    }

    pub fn with_ask(mut self, ask: impl Into<String>) -> Self {
        self.ask = Some(ask.into());
        self
    }

    pub fn with_model(mut self, model: Arc<dyn AgentHostModel>) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_note(mut self, note: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.note = Arc::new(note);
        self
    }

    /// A line of progress, shown as the host shows its agents' work.
    pub fn note(&self, line: &str) {
        (self.note)(line)
    }

    /// The host's model, or `NoModel` naming the package that asked for it.
    pub fn model_for(&self, word: &str) -> Result<Arc<dyn AgentHostModel>, AgentHostError> {
        self.model.clone().ok_or_else(|| AgentHostError::NoModel {
            word: word.to_owned(),
        })
    }
}
