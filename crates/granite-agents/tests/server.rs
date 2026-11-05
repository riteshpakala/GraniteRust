//! A runner's one call, over a channel in memory: every way it can end, and the exit it ends with.

mod common;

use std::sync::Arc;

use granite_agents::{
    AgentChannel, AgentServer, AgentWire, Exit, Failure, MemoryChannel, ModelAnswer, Perform,
    RefusalKind, Roster, roster,
};
use serde_json::{Value, json};

fn perform(word: &str, input: Value, model: bool) -> AgentWire {
    AgentWire::Perform(Perform {
        word: word.into(),
        input,
        directory: Some("/p".into()),
        ask: None,
        model,
    })
}

/// Serves one call: says `messages` after the hello, closes when told, answers each ask with `answers`, and
/// collects what the runner said until its last message.
async fn call(
    roster: Roster,
    messages: Vec<AgentWire>,
    close: bool,
    answers: Vec<Value>,
) -> (Exit, Vec<AgentWire>) {
    let (host, runner) = MemoryChannel::pair();
    let serving = tokio::spawn(AgentServer::serve(runner as Arc<dyn AgentChannel>, roster));
    let mut said = vec![host.recv().await.expect("a hello")];
    for message in messages {
        host.send(&message).unwrap();
    }
    if close {
        host.close();
    }
    let mut answers = answers.into_iter();
    loop {
        let Some(message) = host.recv().await else {
            break;
        };
        if let AgentWire::Model(ask) = &message {
            let answer = ModelAnswer {
                id: ask.id,
                object: answers.next(),
                ..Default::default()
            };
            let _ = host.send(&AgentWire::Answer(answer));
        }
        let last = matches!(message, AgentWire::Output(_) | AgentWire::Failed(_));
        said.push(message);
        if last {
            break;
        }
    }
    (serving.await.unwrap(), said)
}

fn last(said: &[AgentWire]) -> &AgentWire {
    said.last().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_is_answered_once_after_a_hello_that_names_the_answer_standard() {
    let (exit, said) = call(
        roster![common::Echo],
        vec![perform("echo", json!({"text": "hi"}), false)],
        true,
        vec![],
    )
    .await;
    assert_eq!(exit, Exit::Answered);
    let AgentWire::Hello(hello) = &said[0] else {
        panic!("{said:?}")
    };
    assert_eq!(
        (hello.wire.as_str(), hello.answer.as_deref()),
        ("granite-agent/1", Some("granite-answer/1"))
    );
    let AgentWire::Output(output) = last(&said) else {
        panic!("{said:?}")
    };
    assert_eq!(output["said"], "hi");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_model_is_asked_across_the_channel() {
    let (exit, said) = call(
        roster![common::Hosted],
        vec![perform("hosted", json!({"ask_model": true}), true)],
        false,
        vec![json!({"n": 4})],
    )
    .await;
    assert_eq!(exit, Exit::Answered);
    assert!(
        said.iter()
            .any(|message| matches!(message, AgentWire::Model(_)))
    );
    assert!(said.contains(&AgentWire::Note("offered".into())));
    let AgentWire::Output(output) = last(&said) else {
        panic!("{said:?}")
    };
    assert_eq!(
        (output["n"].clone(), output["directory"].clone()),
        (json!(4), json!("/p"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_that_needs_a_model_the_host_does_not_offer_is_refused() {
    let (exit, said) = call(
        roster![common::Hosted],
        vec![perform("hosted", json!({"ask_model": true}), false)],
        false,
        vec![],
    )
    .await;
    assert_eq!(exit, Exit::Refused);
    let AgentWire::Failed(Failure {
        refusal: Some(refusal),
        ..
    }) = last(&said)
    else {
        panic!("{said:?}")
    };
    assert_eq!(refusal.kind, RefusalKind::Model);
}

#[tokio::test(flavor = "multi_thread")]
async fn input_that_does_not_read_is_refused_as_input_with_exit_three() {
    let (exit, said) = call(
        roster![common::Echo],
        vec![perform("echo", json!({"times": 1}), false)],
        false,
        vec![],
    )
    .await;
    assert_eq!(exit, Exit::Refused);
    let AgentWire::Failed(Failure {
        message,
        refusal: Some(refusal),
    }) = last(&said)
    else {
        panic!("{said:?}")
    };
    assert_eq!(refusal.kind, RefusalKind::Input);
    assert!(
        message.starts_with("echo could not read its input"),
        "{message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_word_fails_and_says_what_the_process_carries() {
    let (exit, said) = call(
        roster![common::Echo, common::Maker],
        vec![perform("parrot", json!({}), false)],
        false,
        vec![],
    )
    .await;
    assert_eq!(exit, Exit::Failed);
    assert_eq!(
        *last(&said),
        AgentWire::Failed(Failure::said(
            "no agent is called `parrot` in this process; it carries `echo`, `maker`"
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_ends_the_call_with_one_thirty() {
    let (exit, said) = call(
        roster![common::Echo],
        vec![
            perform("echo", json!({"text": "slow", "delay": 5}), false),
            AgentWire::Cancel,
        ],
        false,
        vec![],
    )
    .await;
    assert_eq!(exit, Exit::Cancelled);
    assert_eq!(*last(&said), AgentWire::Failed(Failure::said("cancelled")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_speaks_out_of_turn_or_leaves_ends_the_process_with_two() {
    let (exit, said) = call(
        roster![common::Echo],
        vec![AgentWire::Note("hi".into())],
        false,
        vec![],
    )
    .await;
    assert_eq!(exit, Exit::Unreadable);
    assert_eq!(
        *last(&said),
        AgentWire::Failed(Failure::said(
            "the host said `note`, which is not a host's message"
        ))
    );

    let (exit, said) = call(roster![common::Echo], vec![], true, vec![]).await;
    assert_eq!(exit, Exit::Unreadable);
    assert_eq!(
        *last(&said),
        AgentWire::Failed(Failure::said("the host left before making a call"))
    );

    let slow = perform("echo", json!({"text": "slow", "delay": 5}), false);
    let (exit, said) = call(
        roster![common::Echo],
        vec![slow.clone(), slow],
        false,
        vec![],
    )
    .await;
    assert_eq!(exit, Exit::Unreadable);
    assert_eq!(
        *last(&said),
        AgentWire::Failed(Failure::said(
            "a process answers one call; the host sent a second `perform`"
        ))
    );
}
