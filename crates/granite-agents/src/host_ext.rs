//! Asking the host's model with a schema rather than its bytes.

use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{AgentHostError, AgentHostModel, GraniteSchema, json};

/// A JSON object that fits `schema` (JSON Schema).
pub async fn object_value(
    model: &dyn AgentHostModel,
    schema: &Value,
    system: &str,
    user: &str,
    max_tokens: u32,
    seconds: f64,
) -> Result<Value, AgentHostError> {
    let schema = json::sorted(schema).map_err(|error| {
        AgentHostError::NoAnswer(format!("the schema could not be written: {error}"))
    })?;
    let bytes = model
        .object(
            schema,
            system.to_owned(),
            user.to_owned(),
            max_tokens,
            seconds,
        )
        .await?;
    serde_json::from_slice(&bytes).map_err(|error| AgentHostError::Answer(error.to_string()))
}

/// A `T`, its schema read off its own deserialization and said beside its fields with `notes`; `Answer` when
/// the model's JSON does not decode as one.
pub async fn object_as<T: DeserializeOwned>(
    model: &dyn AgentHostModel,
    notes: &BTreeMap<String, String>,
    system: &str,
    user: &str,
    max_tokens: u32,
    seconds: f64,
) -> Result<T, AgentHostError> {
    let schema = GraniteSchema::of::<T>().json_schema(notes, &BTreeSet::new());
    let value = object_value(model, &schema, system, user, max_tokens, seconds).await?;
    serde_json::from_value(value).map_err(|error| AgentHostError::Answer(error.to_string()))
}
