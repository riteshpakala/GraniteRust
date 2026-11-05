//! JSON as the standard writes it: every object's keys sorted, slashes never escaped, an integral number with
//! no decimal point where Swift writes one.
//!
//! PIN: SORTED BY CONSTRUCTION. Lines are written from a value rebuilt with sorted keys, never from
//! `serde_json::Map`'s own order, so a dependency that turns on `preserve_order` cannot change the bytes.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// Any JSON a call carries: an input, an output, a schema.
pub type AgentJson = Value;

/// `value` with every object's keys sorted, at every depth.
pub fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, canonical(value));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// `value` as one line of sorted JSON, its newline included.
pub fn line<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = sorted(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// `value` as sorted JSON, with no newline.
pub fn sorted<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&canonical(serde_json::to_value(value)?))
}

/// `value` as sorted JSON, indented: how a record on disk is written.
pub fn pretty<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(&canonical(serde_json::to_value(value)?))
}

/// JSON, or none when the bytes are not.
pub fn parse(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(bytes).ok()
}

/// `serialize_with`: an `f64` that is a whole number is written as an integer (`300`, not `300.0`), as Swift
/// writes a `Double`.
pub fn integral_number<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

/// `deserialize_with`: a key that is there is `Some`, even when its value is `null`.
pub fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
