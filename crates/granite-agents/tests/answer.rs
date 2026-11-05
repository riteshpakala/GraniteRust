//! granite-answer/1: a conforming answer checked and stamped; each way to break it refused as kind `answer`.

mod common;

use granite_agents::{
    AgentHost, AgentPackageError, AgentRefusal, AnswerCheck, AnyAgentPackage, Entry, RefusalKind,
    RosterEntry,
};
use serde_json::Value;

async fn call(
    mode: &str,
    folder: &std::path::Path,
) -> Result<serde_json::Map<String, Value>, granite_agents::BoxError> {
    let input = serde_json::json!({"name": "ada", "mode": mode, "directory": folder}).to_string();
    let host = AgentHost::none().with_directory(folder.to_string_lossy());
    let bytes = AnyAgentPackage::of::<common::Maker>()
        .perform(input.as_bytes(), host)
        .await?;
    match serde_json::from_slice(&bytes)? {
        Value::Object(object) => Ok(object),
        other => panic!("not an object: {other}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conforming_answer_is_checked_and_stamped() {
    let folder = tempfile::tempdir().unwrap();
    let answer = call("good", folder.path()).await.unwrap();
    assert_eq!(answer["format"], "granite-answer/1");
    assert_eq!(answer["agent"], serde_json::json!({"word": "maker"}));
    assert_eq!(answer["card"], "ada", "the package's own keys pass through");
    assert_eq!(answer["made"][0]["kind"], "card");
}

#[tokio::test(flavor = "multi_thread")]
async fn each_way_to_break_the_standard_is_refused_as_an_answer() {
    let rows = [
        (
            "path",
            "`path` cards/elsewhere.card.json is not one of `paths`",
        ),
        ("kind", "`made[0]` is a `chart`, and maker makes `card`"),
        (
            "payload",
            "`payloads[0]` says `test-card/1`, and the file's own `schema` is `other-card/1`",
        ),
        (
            "stamp",
            "`format` is the runner's to say: an answer does not encode it",
        ),
    ];
    for (mode, reason) in rows {
        let folder = tempfile::tempdir().unwrap();
        let error = call(mode, folder.path()).await.expect_err(mode);
        let Some(AgentPackageError::Refused {
            word,
            refusal,
            said,
        }) = error.downcast_ref::<AgentPackageError>()
        else {
            panic!("{mode}: {error}")
        };
        assert_eq!(word, "maker");
        assert_eq!(refusal.kind, RefusalKind::Answer, "{mode}");
        assert!(
            refusal.reasons.iter().any(|said| said == reason),
            "{mode}: {:?}",
            refusal.reasons
        );
        assert!(
            said.starts_with("maker's answer breaks granite-answer/1"),
            "{said}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_returned_refusal_keeps_its_kind_past_the_engine() {
    let folder = tempfile::tempdir().unwrap();
    let error = call("refuse", folder.path()).await.unwrap_err();
    let Some(AgentPackageError::Refused { refusal, said, .. }) =
        error.downcast_ref::<AgentPackageError>()
    else {
        panic!("{error}")
    };
    assert_eq!(
        *refusal,
        AgentRefusal::one(RefusalKind::Call, "`name` is empty: give the card a name")
    );
    assert_eq!(
        said, "`name` is empty: give the card a name",
        "the agent's sentence, not the engine's"
    );
}

#[test]
fn paths_are_compared_as_files() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path().to_string_lossy().into_owned();
    let absolute = folder
        .path()
        .join("cards/a.json")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        AnswerCheck::resolved("cards/a.json", Some(&base)),
        AnswerCheck::resolved(&absolute, None)
    );
    assert_eq!(
        AnswerCheck::resolved("./cards/../cards/a.json", Some(&base)),
        AnswerCheck::resolved(&absolute, Some("/elsewhere"))
    );
    if cfg!(target_os = "macos") {
        assert_eq!(
            AnswerCheck::resolved("/tmp/x/y", None),
            "/private/tmp/x/y",
            "a symlinked ancestor is resolved"
        );
    }
}

#[test]
fn an_entry_says_what_it_makes_and_leaves_out_what_is_empty() {
    let entry = Entry::of(&RosterEntry::Package(AnyAgentPackage::of::<common::Maker>()));
    assert_eq!(
        entry.makes.as_ref().map(|makes| makes[0].kind.as_str()),
        Some("card")
    );
    let read: Entry = serde_json::from_value(serde_json::to_value(&entry).unwrap()).unwrap();
    assert_eq!(read, entry);
    let echo = serde_json::to_value(Entry::of(&RosterEntry::Package(AnyAgentPackage::of::<
        common::Echo,
    >())))
    .unwrap();
    assert!(
        echo.get("makes").is_none(),
        "an agent that declares nothing says nothing"
    );
    assert!(echo.get("guide").is_none());
    assert_eq!(echo["lanes"], 2);
    let cost: granite_agents::AgentProcessCost =
        serde_json::from_str(r#"{"seconds":1,"cpuSeconds":1,"peakBytes":1,"status":"exit 0"}"#)
            .unwrap();
    assert_eq!(cost.asks, 0, "a cost recorded before asks were counted");
}
