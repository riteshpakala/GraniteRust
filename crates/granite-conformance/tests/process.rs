//! A runner in a process of its own, driven over pipes: what it carries, one call, a refusal, a fault, a crash,
//! a cancel, and what each took.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use granite_agents::{
    AgentHost, AgentHostError, AgentHostModel, AgentProcess, AgentProcessCost, AgentRemoteError,
    BoxFuture, RefusalKind,
};
use serde_json::{Value, json};

const PARROT: &str = env!("CARGO_BIN_EXE_parrot-runner");

struct Three;
impl AgentHostModel for Three {
    fn object(
        &self,
        _: Vec<u8>,
        _: String,
        _: String,
        _: u32,
        _: f64,
    ) -> BoxFuture<'_, Result<Vec<u8>, AgentHostError>> {
        Box::pin(async { Ok(br#"{"n":3}"#.to_vec()) })
    }
    fn text(
        &self,
        _: String,
        _: String,
        _: u32,
        _: f64,
    ) -> BoxFuture<'_, Result<String, AgentHostError>> {
        Box::pin(async { Ok(String::new()) })
    }
}

fn noting() -> (AgentHost, Arc<Mutex<Vec<String>>>) {
    let notes = Arc::new(Mutex::new(Vec::new()));
    let kept = notes.clone();
    (
        AgentHost::none().with_note(move |line| kept.lock().unwrap().push(line.to_owned())),
        notes,
    )
}

async fn call(
    input: Value,
    host: &AgentHost,
) -> (Result<Vec<u8>, AgentRemoteError>, Option<AgentProcessCost>) {
    let cost = Arc::new(Mutex::new(None));
    let kept = cost.clone();
    let result = AgentProcess::new(PARROT)
        .perform(
            "parrot",
            input.to_string().as_bytes(),
            host,
            Some(Box::new(move |cost| *kept.lock().unwrap() = Some(cost))),
        )
        .await;
    let cost = cost.lock().unwrap().clone();
    (result, cost)
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_says_what_it_carries_and_makes_nothing() {
    let entries = AgentProcess::new(PARROT).describe().await.unwrap();
    assert_eq!(entries.len(), 1);
    let parrot = &entries[0];
    assert_eq!(
        (parrot.word.as_str(), parrot.module.as_str()),
        ("parrot", "granite_conformance")
    );
    let fields: Vec<&str> = parrot
        .schema
        .as_ref()
        .unwrap()
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(
        fields,
        [
            "text",
            "notes",
            "askModel",
            "delay",
            "crash",
            "fail",
            "fault",
            "directory"
        ]
    );
    assert_eq!(
        parrot.guide.as_deref(),
        Some("Give it text; it says it back.")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_call_is_answered_its_notes_heard_its_ask_answered_and_its_cost_measured() {
    let folder = tempfile::tempdir().unwrap();
    let (host, notes) = noting();
    let host = host
        .with_directory(folder.path().to_string_lossy())
        .with_ask("say hi")
        .with_model(Arc::new(Three));
    let (result, cost) = call(json!({"text": "hi", "notes": 2, "askModel": true}), &host).await;
    let answer: Value = serde_json::from_slice(&result.unwrap()).unwrap();
    assert_eq!(answer["said"], "hi");
    assert_eq!(answer["modelSaid"], 3);
    assert_eq!(answer["ask"], "say hi");
    assert_eq!(
        answer["hostDirectory"],
        folder.path().to_string_lossy().as_ref()
    );
    assert_eq!(answer["format"], "granite-answer/1");
    let pid = answer["pid"].as_u64().unwrap() as i32;
    assert_ne!(pid as u32, std::process::id());
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "the process is gone once it answered"
    );
    let cost = cost.unwrap();
    assert_eq!(cost.status, "exit 0");
    assert_eq!(cost.asks, 1);
    assert!(cost.peak_bytes > 1_000_000, "{}", cost.peak_bytes);
    assert!(
        cost.sentence().starts_with("released · peak "),
        "{}",
        cost.sentence()
    );
    assert_eq!(*notes.lock().unwrap(), ["note 1", "note 2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_comes_back_with_its_kind_and_exit_three() {
    let (result, cost) = call(json!({"text": "x", "fail": true}), &AgentHost::none()).await;
    match result {
        Err(AgentRemoteError::Refused {
            refusal, message, ..
        }) => {
            assert_eq!(refusal.kind, RefusalKind::Call);
            assert_eq!(refusal.reasons, ["told to fail: leave out `fail`"]);
            assert_eq!(message, "told to fail");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cost.unwrap().status, "exit 3");
}

#[tokio::test(flavor = "multi_thread")]
async fn input_that_does_not_read_is_a_refusal_of_kind_input() {
    let (result, cost) = call(json!({"text": 5}), &AgentHost::none()).await;
    let Err(AgentRemoteError::Refused { refusal, .. }) = result else {
        panic!("{result:?}")
    };
    assert_eq!(refusal.kind, RefusalKind::Input);
    assert_eq!(cost.unwrap().status, "exit 3");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fault_is_said_as_the_agents_own() {
    let (result, cost) = call(json!({"text": "x", "fault": true}), &AgentHost::none()).await;
    assert_eq!(
        result.unwrap_err(),
        AgentRemoteError::Fault {
            word: "parrot".into(),
            message: "it broke".into()
        }
    );
    assert_eq!(cost.unwrap().status, "exit 1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_ends_the_call_with_its_signal_and_what_it_printed_off_the_wire() {
    let (result, _) = call(json!({"text": "x", "crash": true}), &AgentHost::none()).await;
    let Err(AgentRemoteError::Ended { status, log, .. }) = result else {
        panic!("{result:?}")
    };
    assert_eq!(status, "signal 6");
    assert!(
        log.contains("this line must not reach the wire"),
        "a println lands on stderr: {log}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_word_is_a_fault_naming_what_it_carries() {
    let result = AgentProcess::new(PARROT)
        .perform("echo", b"{}", &AgentHost::none(), None)
        .await;
    let Err(AgentRemoteError::Fault { message, .. }) = result else {
        panic!("{result:?}")
    };
    assert_eq!(
        message,
        "no agent is called `echo` in this process; it carries `parrot`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_call_ends_its_process() {
    let (host, notes) = noting();
    let waited = tokio::time::timeout(
        Duration::from_millis(1500),
        AgentProcess::new(PARROT).perform("parrot", br#"{"text":"x","delay":30}"#, &host, None),
    )
    .await;
    assert!(waited.is_err());
    let said = notes.lock().unwrap().clone();
    let pid: i32 = said
        .iter()
        .find_map(|line| line.rsplit(' ').next()?.parse().ok())
        .unwrap_or_else(|| panic!("no pid noted: {said:?}"));
    let gone = granite::testing::until(
        || unsafe { libc::kill(pid, 0) } == -1,
        Duration::from_secs(3),
    )
    .await;
    assert!(gone, "process {pid} still runs");
}
