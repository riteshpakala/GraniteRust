//! A package found in its roster and called by its word, JSON in and out, on the engine; what its host offers.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use granite_agents::{
    AgentHost, AgentPackageError, AgentRefusal, AnyAgentPackage, PackageHost, RefusalKind, Roster,
    RosterEntry, roster,
};
use serde_json::{Value, json};

#[test]
fn a_roster_keeps_the_first_of_a_word_and_describes_what_it_carries() {
    let roster = roster![common::Echo, common::Maker, common::Echo];
    assert_eq!(roster.entries.len(), 2);
    let words: Vec<String> = roster
        .describe()
        .into_iter()
        .map(|entry| entry.word)
        .collect();
    assert_eq!(words, ["echo", "maker"]);
    assert!(roster.package("maker").is_some());
    assert!(roster.package("parrot").is_none());
    let again = Roster::new(
        roster
            .entries
            .iter()
            .cloned()
            .chain([RosterEntry::Package(AnyAgentPackage::of::<common::Echo>())]),
    );
    assert_eq!(again.entries.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_is_json_in_and_a_stamped_answer_out() {
    let echo = AnyAgentPackage::of::<common::Echo>();
    let bytes = echo
        .perform(br#"{"text": "hi", "times": 3}"#, AgentHost::none())
        .await
        .unwrap();
    let answer: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(answer["said"], "hi hi hi");
    assert_eq!(answer["format"], "granite-answer/1");
    assert_eq!(answer["agent"]["word"], "echo");
    assert_eq!(answer["paths"], json!([]));
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.find("\"agent\"").unwrap() < text.find("\"said\"").unwrap(),
        "keys sorted: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_are_runs_of_their_own() {
    let echo = AnyAgentPackage::of::<common::Echo>();
    let before = PackageHost::<common::Echo>::shared()
        .center
        .state()
        .finished;
    let (a, b) = tokio::join!(
        echo.perform(br#"{"text": "a", "delay": 0.05}"#, AgentHost::none()),
        echo.perform(br#"{"text": "b", "delay": 0.05}"#, AgentHost::none())
    );
    let said = |bytes: Vec<u8>| serde_json::from_slice::<Value>(&bytes).unwrap()["said"].clone();
    assert_eq!(said(a.unwrap()), "a");
    assert_eq!(said(b.unwrap()), "b");
    assert!(
        PackageHost::<common::Echo>::shared()
            .center
            .state()
            .finished
            >= before + 2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn input_that_does_not_read_is_refused_as_input_and_says_whose() {
    let error = AnyAgentPackage::of::<common::Echo>()
        .perform(br#"{"times": 2}"#, AgentHost::none())
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("echo could not read its input: missing field `text`"),
        "{error}"
    );
    assert_eq!(
        AgentRefusal::of(error.as_ref()).map(|refusal| refusal.kind),
        Some(RefusalKind::Input)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_call_cancels_its_run() {
    let echo = AnyAgentPackage::of::<common::Echo>();
    let before = PackageHost::<common::Echo>::shared()
        .center
        .state()
        .cancelled;
    let waited = tokio::time::timeout(
        Duration::from_millis(50),
        echo.perform(br#"{"text": "slow", "delay": 5}"#, AgentHost::none()),
    )
    .await;
    assert!(waited.is_err());
    let center = PackageHost::<common::Echo>::shared().center.clone();
    assert!(
        granite::testing::until(|| center.state().cancelled > before, Duration::from_secs(2)).await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_reads_what_its_host_offers_and_asks_its_model() {
    let notes = Arc::new(Mutex::new(Vec::new()));
    let kept = notes.clone();
    let model = common::FakeModel::new(vec![json!({"n": 3})]);
    let host = AgentHost::none()
        .with_directory("/p")
        .with_ask("count them")
        .with_model(model.clone())
        .with_note(move |line| kept.lock().unwrap().push(line.to_owned()));
    let bytes = AnyAgentPackage::of::<common::Hosted>()
        .perform(br#"{"ask_model": true}"#, host)
        .await
        .unwrap();
    let answer: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (
            answer["directory"].clone(),
            answer["ask"].clone(),
            answer["n"].clone()
        ),
        (json!("/p"), json!("count them"), json!(3))
    );
    assert_eq!(*notes.lock().unwrap(), ["offered"]);
    assert_eq!(*model.asked.lock().unwrap(), ["How many?"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_needs_a_model_on_a_host_without_one_is_refused_as_model() {
    let error = AnyAgentPackage::of::<common::Hosted>()
        .perform(br#"{"ask_model": true}"#, AgentHost::none())
        .await
        .unwrap_err();
    let Some(AgentPackageError::Refused { refusal, .. }) =
        error.downcast_ref::<AgentPackageError>()
    else {
        panic!("{error}")
    };
    assert_eq!(
        *refusal,
        AgentRefusal::one(
            RefusalKind::Model,
            "hosted asks the host's model, and this host has none loaded"
        )
    );
}
