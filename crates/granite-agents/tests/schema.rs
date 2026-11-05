//! A schema read off an input's own deserialization, as data and as JSON Schema.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use granite_agents::{AnyAgentPackage, Field, GraniteSchema};
use serde::Deserialize;
use serde_json::json;

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Look {
    Sepia,
    BlackAndWhite,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Size {
    width: u32,
    height: Option<u32>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    path: String,
    look: Option<Look>,
    sizes: Option<Vec<u32>>,
    max_side: Option<f64>,
    size: Size,
    extra: Option<serde_json::Value>,
    loud: bool,
    directory: Option<String>,
}

#[test]
fn fields_are_read_in_order_with_what_each_takes() {
    let schema = GraniteSchema::of::<Input>();
    assert_eq!(
        schema,
        GraniteSchema::Object(vec![
            Field::new("path", GraniteSchema::String, true),
            Field::new(
                "look",
                GraniteSchema::Choice(vec!["sepia".into(), "black-and-white".into()]),
                false
            ),
            Field::new(
                "sizes",
                GraniteSchema::Array(Box::new(GraniteSchema::Integer)),
                false
            ),
            Field::new("maxSide", GraniteSchema::Number, false),
            Field::new(
                "size",
                GraniteSchema::Object(vec![
                    Field::new("width", GraniteSchema::Integer, true),
                    Field::new("height", GraniteSchema::Integer, false)
                ]),
                true
            ),
            Field::new("extra", GraniteSchema::Any, false),
            Field::new("loud", GraniteSchema::Boolean, true),
            Field::new("directory", GraniteSchema::String, false),
        ])
    );
}

#[test]
fn a_model_is_shown_json_schema_with_notes_and_without_the_hosts_folder() {
    let notes = BTreeMap::from([("path".to_owned(), "the photo".to_owned())]);
    let hiding = BTreeSet::from(["directory".to_owned()]);
    let shown = GraniteSchema::of::<Input>().json_schema(&notes, &hiding);
    assert_eq!(
        shown["properties"]["path"],
        json!({"type": "string", "description": "the photo"})
    );
    assert_eq!(
        shown["properties"]["look"],
        json!({"type": "string", "enum": ["sepia", "black-and-white"]})
    );
    assert_eq!(shown["properties"]["extra"], json!({}));
    assert!(shown["properties"].get("directory").is_none());
    assert_eq!(shown["required"], json!(["path", "size", "loud"]));
}

#[test]
fn the_schema_as_data_reads_back_exactly() {
    let schema = GraniteSchema::of::<Input>();
    let value = serde_json::to_value(&schema).unwrap();
    assert_eq!(
        value["object"][0],
        json!({"name": "path", "required": true, "schema": "string"})
    );
    assert_eq!(value["object"][5]["schema"], json!("any"));
    assert_eq!(
        serde_json::from_value::<GraniteSchema>(value).unwrap(),
        schema
    );
    let error = serde_json::from_value::<GraniteSchema>(json!("weather"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("no schema is called \"weather\""), "{error}");
    let error = serde_json::from_value::<GraniteSchema>(json!({"rows": []}))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("a schema is a word or one of choice, array, object"),
        "{error}"
    );
}

#[test]
fn a_package_shows_its_input_and_takes_the_hosts_folder() {
    let echo = AnyAgentPackage::of::<common::Echo>();
    let names: Vec<&str> = echo
        .schema
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(names, ["text", "times", "delay", "directory"]);
    assert!(echo.takes_directory());
    assert!(!AnyAgentPackage::of::<common::Hosted>().takes_directory());
    assert_eq!(
        echo.module, "schema",
        "a package's module is its crate's name unless it says one"
    );
    assert_eq!(echo.r#type, "schema.Echo");
}
