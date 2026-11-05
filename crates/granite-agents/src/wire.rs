//! The messages of `granite-agent/1`: JSON objects, one per line, each named by its one key.
//!
//! ```text
//! runner → host   {"hello":{"answer":"granite-answer/1","pid":4512,"wire":"granite-agent/1"}}
//! host → runner   {"perform":{"ask":"…","directory":"/p","input":{…},"model":true,"word":"plate"}}
//! runner → host   {"note":"round 1: measured clean"}
//! runner → host   {"model":{"id":1,"kind":"object","maxTokens":3000,"schema":{…},"seconds":300,"system":"…","user":"…"}}
//! host → runner   {"answer":{"id":1,"object":{…}}}
//! runner → host   {"output":{…}}   or   {"failed":"a sentence","refused":{"kind":"call","reasons":[…]}}
//! host → runner   {"cancel":{}}
//! runner → host   {"entries":[…]}  (only with --describe)
//! ```
//!
//! PIN: UNKNOWN FIELDS ARE IGNORED, UNKNOWN MESSAGES ARE NOT. A line whose one key is none of these does not
//! read; a reader drops it and says so.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{AgentRefusal, Entry, json};

/// The runner's first message.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub wire: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    /// `granite-answer/1` from a runner whose calls keep the answer standard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

impl Default for Hello {
    fn default() -> Self {
        Hello {
            wire: AgentWire::VERSION.to_owned(),
            pid: None,
            answer: None,
        }
    }
}

/// The one call a process answers.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Perform {
    pub word: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<String>,
    /// Whether the host offers a model.
    pub model: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskKind {
    Object,
    Text,
}

/// An ask of the host's model.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelAsk {
    pub id: u64,
    pub kind: AskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    #[serde(serialize_with = "json::integral_number")]
    pub seconds: f64,
}

/// The host's answer to an ask: exactly one of `object`, `text` or `error`.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct ModelAnswer {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Why a call failed: a sentence, and its refusal when it was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Failure {
    pub message: String,
    pub refusal: Option<AgentRefusal>,
}

impl Failure {
    pub fn said(message: impl Into<String>) -> Self {
        Failure {
            message: message.into(),
            refusal: None,
        }
    }

    pub fn refused(message: impl Into<String>, refusal: AgentRefusal) -> Self {
        Failure {
            message: message.into(),
            refusal: Some(refusal),
        }
    }
}

/// One message.
#[derive(Clone, PartialEq, Debug)]
pub enum AgentWire {
    Hello(Hello),
    Perform(Perform),
    Note(String),
    Model(ModelAsk),
    Answer(ModelAnswer),
    Output(Value),
    Failed(Failure),
    Cancel,
    Entries(Vec<Entry>),
}

/// A line that is not a message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WireError(pub String);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WireError {}

impl AgentWire {
    pub const VERSION: &'static str = "granite-agent/1";
    pub const MAJOR: u32 = 1;
    /// Every message, in the order a call says them.
    pub const KINDS: [&'static str; 9] = [
        "hello", "perform", "note", "model", "answer", "output", "failed", "cancel", "entries",
    ];

    pub fn kind(&self) -> &'static str {
        match self {
            AgentWire::Hello(_) => "hello",
            AgentWire::Perform(_) => "perform",
            AgentWire::Note(_) => "note",
            AgentWire::Model(_) => "model",
            AgentWire::Answer(_) => "answer",
            AgentWire::Output(_) => "output",
            AgentWire::Failed(_) => "failed",
            AgentWire::Cancel => "cancel",
            AgentWire::Entries(_) => "entries",
        }
    }

    /// The message as one line of sorted JSON, newline included.
    pub fn line(&self) -> Result<Vec<u8>, WireError> {
        let encode = |value: Result<Value, serde_json::Error>| {
            value.map_err(|error| WireError(error.to_string()))
        };
        let mut map = Map::new();
        match self {
            AgentWire::Hello(hello) => {
                map.insert("hello".into(), encode(serde_json::to_value(hello))?)
            }
            AgentWire::Perform(perform) => {
                map.insert("perform".into(), encode(serde_json::to_value(perform))?)
            }
            AgentWire::Note(note) => map.insert("note".into(), Value::String(note.clone())),
            AgentWire::Model(ask) => map.insert("model".into(), encode(serde_json::to_value(ask))?),
            AgentWire::Answer(answer) => {
                map.insert("answer".into(), encode(serde_json::to_value(answer))?)
            }
            AgentWire::Output(output) => map.insert("output".into(), output.clone()),
            AgentWire::Failed(failure) => {
                if let Some(refusal) = &failure.refusal {
                    map.insert("refused".into(), encode(serde_json::to_value(refusal))?);
                }
                map.insert("failed".into(), Value::String(failure.message.clone()))
            }
            AgentWire::Cancel => map.insert("cancel".into(), Value::Object(Map::new())),
            AgentWire::Entries(entries) => {
                map.insert("entries".into(), encode(serde_json::to_value(entries))?)
            }
        };
        json::line(&Value::Object(map)).map_err(|error| WireError(error.to_string()))
    }

    /// The message one line holds. Keys are tried in Swift Granite's order; a field a reader does not know is
    /// ignored, and a malformed `refused` is dropped, leaving the sentence.
    pub fn read(line: &[u8]) -> Result<AgentWire, WireError> {
        let value: Value =
            serde_json::from_slice(line).map_err(|error| WireError(error.to_string()))?;
        let Value::Object(mut map) = value else {
            return Err(WireError("a message is a JSON object".into()));
        };
        fn take<T: serde::de::DeserializeOwned>(value: Value, key: &str) -> Result<T, WireError> {
            serde_json::from_value(value).map_err(|error| WireError(format!("`{key}`: {error}")))
        }
        let present = |map: &Map<String, Value>, key: &str| {
            map.get(key).is_some_and(|value| !value.is_null())
        };
        if present(&map, "hello") {
            return Ok(AgentWire::Hello(take(
                map.remove("hello").unwrap(),
                "hello",
            )?));
        }
        if present(&map, "perform") {
            return Ok(AgentWire::Perform(take(
                map.remove("perform").unwrap(),
                "perform",
            )?));
        }
        if present(&map, "note") {
            return Ok(AgentWire::Note(take(map.remove("note").unwrap(), "note")?));
        }
        if present(&map, "model") {
            return Ok(AgentWire::Model(take(
                map.remove("model").unwrap(),
                "model",
            )?));
        }
        if present(&map, "answer") {
            return Ok(AgentWire::Answer(take(
                map.remove("answer").unwrap(),
                "answer",
            )?));
        }
        if let Some(output) = map.remove("output") {
            return Ok(AgentWire::Output(output));
        }
        if present(&map, "failed") {
            let message: String = take(map.remove("failed").unwrap(), "failed")?;
            let refusal = map
                .remove("refused")
                .and_then(|refused| serde_json::from_value(refused).ok());
            return Ok(AgentWire::Failed(Failure { message, refusal }));
        }
        if map.contains_key("cancel") {
            return Ok(AgentWire::Cancel);
        }
        if present(&map, "entries") {
            return Ok(AgentWire::Entries(take(
                map.remove("entries").unwrap(),
                "entries",
            )?));
        }
        Err(WireError(format!(
            "a message is one of {}",
            AgentWire::KINDS.join(", ")
        )))
    }
}
