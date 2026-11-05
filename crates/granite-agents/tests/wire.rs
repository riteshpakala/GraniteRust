//! The messages of granite-agent/1, as lines.

use granite_agents::{AgentHostError, json};
use granite_agents::{
    AgentPackageError, AgentRefusal, AgentWire, AskKind, Entry, EntryKind, Failure, GraniteSchema,
    Hello, ModelAnswer, ModelAsk, Perform, RefusalKind,
};
use serde_json::json;

fn every_message() -> Vec<AgentWire> {
    vec![
        AgentWire::Hello(Hello {
            pid: Some(7),
            answer: Some("granite-answer/1".into()),
            ..Default::default()
        }),
        AgentWire::Perform(Perform {
            word: "plate".into(),
            input: json!({"sources": ["Sources"]}),
            directory: Some("/p".into()),
            ask: Some("make a graphic".into()),
            model: true,
        }),
        AgentWire::Note("round 1: measured clean".into()),
        AgentWire::Model(ModelAsk {
            id: 1,
            kind: AskKind::Object,
            schema: Some(json!({"type": "object"})),
            system: "s".into(),
            user: "u".into(),
            max_tokens: 3000,
            seconds: 300.0,
        }),
        AgentWire::Answer(ModelAnswer {
            id: 1,
            object: Some(json!({"n": 3})),
            ..Default::default()
        }),
        AgentWire::Output(json!({"format": "granite-answer/1", "paths": []})),
        AgentWire::Failed(Failure::refused(
            "told to fail",
            AgentRefusal::one(RefusalKind::Call, "leave out `fail`"),
        )),
        AgentWire::Cancel,
        AgentWire::Entries(vec![Entry {
            kind: EntryKind::Package,
            word: "echo".into(),
            summary: "says it back.".into(),
            module: "Echo".into(),
            r#type: "Echo.Echo".into(),
            schema: Some(GraniteSchema::Object(vec![granite_agents::Field::new(
                "text",
                GraniteSchema::String,
                true,
            )])),
            fields: None,
            lanes: Some(2),
            guide: None,
            makes: None,
            tool_names: None,
            budget: None,
            shell: None,
            report_rule: None,
        }]),
    ]
}

#[test]
fn hello_is_the_golden_line() {
    let line = AgentWire::Hello(Hello {
        pid: Some(7),
        ..Default::default()
    })
    .line()
    .unwrap();
    assert_eq!(
        String::from_utf8(line).unwrap(),
        "{\"hello\":{\"pid\":7,\"wire\":\"granite-agent/1\"}}\n"
    );
}

#[test]
fn every_message_is_one_line_that_reads_back() {
    for message in every_message() {
        let line = message.line().unwrap();
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(
            line.iter().filter(|byte| **byte == b'\n').count(),
            1,
            "{}",
            message.kind()
        );
        assert_eq!(
            AgentWire::read(&line[..line.len() - 1]).unwrap(),
            message,
            "{}",
            message.kind()
        );
    }
    assert_eq!(
        AgentWire::KINDS,
        [
            "hello", "perform", "note", "model", "answer", "output", "failed", "cancel", "entries"
        ]
    );
}

#[test]
fn keys_are_sorted_slashes_kept_and_whole_seconds_are_integers() {
    let line = String::from_utf8(every_message()[3].line().unwrap()).unwrap();
    assert_eq!(
        line,
        "{\"model\":{\"id\":1,\"kind\":\"object\",\"maxTokens\":3000,\"schema\":{\"type\":\"object\"},\"seconds\":300,\"system\":\"s\",\"user\":\"u\"}}\n"
    );
    let perform = String::from_utf8(every_message()[1].line().unwrap()).unwrap();
    assert!(perform.contains("\"directory\":\"/p\""), "{perform}");
    let unsorted = json!({"b": 1, "a": {"d": 2, "c": 3}});
    assert_eq!(
        String::from_utf8(json::sorted(&unsorted).unwrap()).unwrap(),
        "{\"a\":{\"c\":3,\"d\":2},\"b\":1}"
    );
}

#[test]
fn a_refusal_crosses_the_wire_beside_its_sentence() {
    let refusal = AgentRefusal::new(
        RefusalKind::Call,
        ["`post` is mapped twice", "`Comment` has no `text`"],
    );
    let message = AgentWire::Failed(Failure::refused(
        "2 problems with the call",
        refusal.clone(),
    ));
    let line = message.line().unwrap();
    let text = String::from_utf8(line.clone()).unwrap();
    assert!(
        text.contains("\"failed\":\"2 problems with the call\""),
        "{text}"
    );
    assert_eq!(AgentWire::read(&line).unwrap(), message);
    assert_eq!(
        AgentWire::read(br#"{"failed":"it broke"}"#).unwrap(),
        AgentWire::Failed(Failure::said("it broke"))
    );
    let newer =
        AgentWire::read(br#"{"failed":"x","refused":{"kind":"weather","reasons":["rain"]}}"#)
            .unwrap();
    assert_eq!(
        newer,
        AgentWire::Failed(Failure::refused(
            "x",
            AgentRefusal::one(RefusalKind::Host, "rain")
        )),
        "an unknown kind reads as host"
    );
    let broken = AgentWire::read(br#"{"failed":"x","refused":7}"#).unwrap();
    assert_eq!(
        broken,
        AgentWire::Failed(Failure::said("x")),
        "a malformed refusal is dropped, the sentence kept"
    );
    assert_eq!(
        refusal.to_string(),
        "2 problems, so nothing was done:\n- `post` is mapped twice\n- `Comment` has no `text`"
    );
}

#[test]
fn unknown_fields_are_ignored_and_unknown_messages_are_not() {
    assert_eq!(
        AgentWire::read(br#"{"hello":{"wire":"granite-agent/1","later":true}}"#).unwrap(),
        AgentWire::Hello(Hello::default())
    );
    let error = AgentWire::read(br#"{"weather":{}}"#).unwrap_err();
    assert_eq!(
        error.0,
        "a message is one of hello, perform, note, model, answer, output, failed, cancel, entries"
    );
    assert_eq!(
        AgentWire::read(br#"{"output":null}"#).unwrap(),
        AgentWire::Output(serde_json::Value::Null),
        "an output of null is an output"
    );
    assert!(
        AgentWire::read(br#"{"perform":{"word":"x"}}"#).is_err(),
        "a perform needs its input and model"
    );
}

#[test]
fn what_is_a_refusal_and_what_is_a_fault() {
    let input = AgentPackageError::Input {
        word: "w".into(),
        reason: "bad".into(),
    };
    assert_eq!(
        AgentRefusal::of(&input),
        Some(AgentRefusal::one(RefusalKind::Input, "bad"))
    );
    assert_eq!(
        AgentRefusal::of(&AgentHostError::NoModel { word: "w".into() }).map(|refusal| refusal.kind),
        Some(RefusalKind::Model)
    );
    assert_eq!(
        AgentRefusal::of(&std::io::Error::other("disk")),
        None,
        "anything else is a fault"
    );
}
