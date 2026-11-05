//! What an input takes, read off its own deserialization: each field's name and kind, whether it must be there,
//! and for a field with a fixed set of values, the values. What a host shows a model as a package's parameters.
//!
//! PIN: READ, NOT WRITTEN. [`GraniteSchema::of`] runs the type's own `Deserialize` once against a recorder
//! ([`probe`]): an `Option` field is optional, every other field is required (a `#[serde(default)]` field too,
//! as a Swift default does), a unit-variant enum is a choice of its serialized names, and the fields are kept in
//! the order the type declares them. A type whose `Deserialize` is written by hand says its own schema
//! (`AgentPackage::schema`).
//! PIN: NOT JSON SCHEMA. This form crosses processes and reads back exactly (`"any"`, field order);
//! [`GraniteSchema::json_schema`] is what a model is shown.

mod probe;

use std::collections::{BTreeMap, BTreeSet};

use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GraniteSchema {
    String,
    Integer,
    Number,
    Boolean,
    /// One of these strings: an enum of unit variants.
    Choice(Vec<String>),
    Array(Box<GraniteSchema>),
    Object(Vec<Field>),
    /// A value whose deserialization said nothing the recorder could read.
    Any,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub schema: GraniteSchema,
    pub required: bool,
}

impl Field {
    pub fn new(name: &str, schema: GraniteSchema, required: bool) -> Self {
        Field {
            name: name.to_owned(),
            schema,
            required,
        }
    }
}

impl GraniteSchema {
    /// The schema of `T`, recorded by deserializing it once from nothing.
    pub fn of<T: DeserializeOwned>() -> GraniteSchema {
        probe::record::<T>()
    }

    /// An object's fields, in order; none for anything else.
    pub fn fields(&self) -> &[Field] {
        match self {
            GraniteSchema::Object(fields) => fields,
            _ => &[],
        }
    }

    /// As JSON Schema, with `notes` said beside the fields they name and the fields in `hiding` left out.
    pub fn json_schema(
        &self,
        notes: &BTreeMap<String, String>,
        hiding: &BTreeSet<String>,
    ) -> Value {
        match self {
            GraniteSchema::String => json!({"type": "string"}),
            GraniteSchema::Integer => json!({"type": "integer"}),
            GraniteSchema::Number => json!({"type": "number"}),
            GraniteSchema::Boolean => json!({"type": "boolean"}),
            GraniteSchema::Choice(values) => json!({"type": "string", "enum": values}),
            GraniteSchema::Array(element) => {
                json!({"type": "array", "items": element.json_schema(&BTreeMap::new(), &BTreeSet::new())})
            }
            GraniteSchema::Object(fields) => {
                let shown: Vec<&Field> = fields
                    .iter()
                    .filter(|field| !hiding.contains(&field.name))
                    .collect();
                let mut properties = Map::new();
                for field in &shown {
                    let mut property = field.schema.json_schema(&BTreeMap::new(), &BTreeSet::new());
                    if let (Some(note), Value::Object(property)) =
                        (notes.get(&field.name), &mut property)
                    {
                        property.insert("description".into(), Value::String(note.clone()));
                    }
                    properties.insert(field.name.clone(), property);
                }
                let required: Vec<&str> = shown
                    .iter()
                    .filter(|field| field.required)
                    .map(|field| field.name.as_str())
                    .collect();
                json!({"type": "object", "properties": properties, "required": required})
            }
            GraniteSchema::Any => json!({}),
        }
    }
}

impl Serialize for GraniteSchema {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}

impl GraniteSchema {
    /// The schema as data: a word, or one key naming its shape.
    pub fn to_value(&self) -> Value {
        match self {
            GraniteSchema::String => json!("string"),
            GraniteSchema::Integer => json!("integer"),
            GraniteSchema::Number => json!("number"),
            GraniteSchema::Boolean => json!("boolean"),
            GraniteSchema::Any => json!("any"),
            GraniteSchema::Choice(values) => json!({"choice": values}),
            GraniteSchema::Array(element) => json!({"array": element.to_value()}),
            GraniteSchema::Object(fields) => json!({"object": fields
                .iter()
                .map(|field| json!({"name": field.name, "required": field.required, "schema": field.schema.to_value()}))
                .collect::<Vec<_>>()}),
        }
    }
}

impl<'de> Deserialize<'de> for GraniteSchema {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        GraniteSchema::from_value(value).map_err(de::Error::custom)
    }
}

impl GraniteSchema {
    fn from_value(value: Value) -> Result<GraniteSchema, String> {
        match value {
            Value::String(word) => match word.as_str() {
                "string" => Ok(GraniteSchema::String),
                "integer" => Ok(GraniteSchema::Integer),
                "number" => Ok(GraniteSchema::Number),
                "boolean" => Ok(GraniteSchema::Boolean),
                "any" => Ok(GraniteSchema::Any),
                other => Err(format!("no schema is called \"{other}\"")),
            },
            Value::Object(mut map) => {
                if let Some(values) = map.remove("choice") {
                    return serde_json::from_value(values)
                        .map(GraniteSchema::Choice)
                        .map_err(|error| error.to_string());
                }
                if let Some(element) = map.remove("array") {
                    return GraniteSchema::from_value(element)
                        .map(|element| GraniteSchema::Array(Box::new(element)));
                }
                if let Some(fields) = map.remove("object") {
                    return serde_json::from_value(fields)
                        .map(GraniteSchema::Object)
                        .map_err(|error| error.to_string());
                }
                Err("a schema is a word or one of choice, array, object".into())
            }
            _ => Err("a schema is a word or one of choice, array, object".into()),
        }
    }
}
