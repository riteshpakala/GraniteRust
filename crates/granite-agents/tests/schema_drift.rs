//! The code held to the standard it implements: `Schema/` at the repo root, copied from Granite. Every message,
//! entry, answer and installation this crate writes validates against its JSON Schema, and `GraniteAgent.md`
//! names every message and key the code has.

mod common;

use std::path::PathBuf;

use granite_agents::{
    AgentHost, AgentInstallation, AgentSource, AgentWire, AnyAgentPackage, Entry, Failure, Hello,
    RosterEntry,
};
use serde_json::{Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: PathBuf) -> Value {
    serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
    )
    .unwrap()
}

/// A validator for `file`, every schema of the standard and Craft's registered by its `$id`, nothing fetched.
fn validator(file: &str) -> jsonschema::Validator {
    let mut options = jsonschema::options();
    for folder in [root().join("Schema"), root().join("containers/schemas")] {
        for entry in std::fs::read_dir(folder).unwrap().flatten() {
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let schema = read(entry.path());
                let id = schema["$id"].as_str().unwrap().to_owned();
                options.with_resource(id, jsonschema::Resource::from_contents(schema).unwrap());
            }
        }
    }
    let path = if file.starts_with("craft-") {
        root().join("containers/schemas").join(file)
    } else {
        root().join("Schema").join(file)
    };
    options.build(&read(path)).unwrap()
}

fn valid(validator: &jsonschema::Validator, instance: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| format!("{error} at {}", error.instance_path))
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:?}\n{instance}");
}

fn as_json(message: &AgentWire) -> Value {
    serde_json::from_slice(&message.line().unwrap()).unwrap()
}

#[test]
fn every_message_validates_against_the_wire_schema() {
    let wire = validator("granite-agent-wire.schema.json");
    let entries = vec![Entry::of(&RosterEntry::Package(AnyAgentPackage::of::<
        common::Maker,
    >()))];
    let messages = [
        AgentWire::Hello(Hello { pid: Some(4512), answer: Some("granite-answer/1".into()), ..Default::default() }),
        AgentWire::read(br#"{"perform":{"ask":"a","directory":"/p","input":{"text":"hi"},"model":true,"word":"echo"}}"#).unwrap(),
        AgentWire::Note("planning".into()),
        AgentWire::read(br#"{"model":{"id":1,"kind":"object","maxTokens":3000,"schema":{"type":"object"},"seconds":300,"system":"s","user":"u"}}"#).unwrap(),
        AgentWire::read(br#"{"answer":{"id":1,"object":{"n":3}}}"#).unwrap(),
        AgentWire::Output(json!({"agent": {"word": "echo"}, "format": "granite-answer/1", "paths": []})),
        AgentWire::Failed(Failure::refused("told", granite_agents::AgentRefusal::one(granite_agents::RefusalKind::Call, "leave it out"))),
        AgentWire::Cancel,
        AgentWire::Entries(entries),
    ];
    for message in &messages {
        valid(&wire, &as_json(message), message.kind());
    }
}

#[test]
fn an_entry_validates_against_the_entry_schema() {
    let schema = validator("granite-agent-entry.schema.json");
    for entry in [
        Entry::of(&RosterEntry::Package(AnyAgentPackage::of::<common::Maker>())),
        Entry::of(&RosterEntry::Package(AnyAgentPackage::of::<common::Echo>())),
    ] {
        valid(&schema, &serde_json::to_value(&entry).unwrap(), &entry.word);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stamped_answer_validates_against_granites_answer_schema_and_crafts_output_schema() {
    let folder = tempfile::tempdir().unwrap();
    let input = json!({"name": "ada", "mode": "good", "directory": folder.path()}).to_string();
    let host = AgentHost::none().with_directory(folder.path().to_string_lossy());
    let bytes = AnyAgentPackage::of::<common::Maker>()
        .perform(input.as_bytes(), host)
        .await
        .unwrap();
    let answer: Value = serde_json::from_slice(&bytes).unwrap();
    valid(
        &validator("granite-agent-answer.schema.json"),
        &answer,
        "granite-answer/1",
    );
    valid(
        &validator("craft-agent-output.schema.json"),
        &answer,
        "craft-agent/1 output",
    );
}

#[test]
fn an_installation_validates_against_the_installation_schema() {
    let installation = AgentInstallation {
        format: AgentInstallation::FORMAT.into(),
        name: "ComicAgent".into(),
        source: AgentSource::Git {
            url: "https://github.com/rao-craft/ComicAgent.git".into(),
            revision: Some("abc123".into()),
        },
        revision: Some("abc123".into()),
        granite: Some("0.1.0@def456".into()),
        executable: "agent".into(),
        sha256: "0".repeat(64),
        size: 1024,
        modified: Some(1_793_427_000.5),
        built_at: granite_agents::install::now_iso8601(),
        entries: vec![Entry::of(&RosterEntry::Package(AnyAgentPackage::of::<
            common::Maker,
        >()))],
    };
    let value = serde_json::to_value(&installation).unwrap();
    valid(
        &validator("granite-agent-installation.schema.json"),
        &value,
        "installation",
    );
    let path = serde_json::to_value(AgentSource::Path { path: "/x".into() }).unwrap();
    assert_eq!(path, json!({"path": "/x"}));
    assert_eq!(
        serde_json::from_value::<AgentInstallation>(value).unwrap(),
        installation
    );
}

#[test]
fn the_standard_names_what_the_code_has() {
    let standard = std::fs::read_to_string(root().join("Schema/GraniteAgent.md")).unwrap();
    for kind in AgentWire::KINDS {
        assert!(
            standard.contains(&format!("`{kind}`")),
            "the standard does not name the message `{kind}`"
        );
    }
    for field in Entry::FIELD_NAMES {
        assert!(
            standard.contains(&format!("`{field}`")),
            "the standard does not name the entry field `{field}`"
        );
    }
    for key in [
        "format",
        "name",
        "source",
        "revision",
        "granite",
        "executable",
        "sha256",
        "size",
        "modified",
        "builtAt",
        "entries",
    ] {
        assert!(
            standard.contains(key),
            "the standard does not name the installation key `{key}`"
        );
    }
}
