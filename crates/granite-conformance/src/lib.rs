//! `parrot`: a package agent that exercises every message of the wire. It says its text back, notes as it goes,
//! asks the host's model when told, waits when told, refuses, faults, and crashes when told.

use std::collections::BTreeMap;
use std::fmt;

use granite_agents::{
    AgentContext, AgentPackage, AgentRefusal, AgentRefusing, Answer, BoxError, RefusalKind,
    host_ext,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Default)]
pub struct Parrot;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParrotInput {
    pub text: String,
    pub notes: Option<u32>,
    pub ask_model: Option<bool>,
    pub delay: Option<f64>,
    pub crash: Option<bool>,
    pub fail: Option<bool>,
    pub fault: Option<bool>,
    pub directory: Option<String>,
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParrotSaid {
    pub said: String,
    pub model_said: Option<i64>,
    pub directory: Option<String>,
    pub host_directory: Option<String>,
    pub ask: Option<String>,
    pub pid: u32,
}

/// A refusal of the call, as an agent's own error says one.
#[derive(Debug)]
struct Told;

impl fmt::Display for Told {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("told to fail")
    }
}

impl std::error::Error for Told {}

impl AgentRefusing for Told {
    fn refusal(&self) -> AgentRefusal {
        AgentRefusal::one(RefusalKind::Call, "told to fail: leave out `fail`")
    }
}

impl AgentPackage for Parrot {
    type Input = ParrotInput;
    type Event = ParrotSaid;
    type Output = Answer<ParrotSaid>;
    type Compute = ();

    const WORD: &'static str = "parrot";
    const SUMMARY: &'static str = "says its text back, in a process of its own.";

    fn fields() -> BTreeMap<String, String> {
        BTreeMap::from([("text".to_owned(), "what to say".to_owned())])
    }

    fn guide() -> String {
        "Give it text; it says it back.".into()
    }

    async fn compute() -> Result<(), BoxError> {
        Ok(())
    }

    fn output(events: Vec<ParrotSaid>) -> Option<Answer<ParrotSaid>> {
        events.last().map(|said| Answer {
            own: said.clone(),
            ..Default::default()
        })
    }

    async fn run(
        &self,
        input: ParrotInput,
        context: AgentContext<(), ParrotSaid>,
    ) -> Result<(), BoxError> {
        println!("this line must not reach the wire");
        for index in 0..input.notes.unwrap_or(0) {
            context.note(format!("note {}", index + 1));
        }
        if input.crash == Some(true) {
            std::process::abort();
        }
        if input.fail == Some(true) {
            return Err(Told.refused());
        }
        if input.fault == Some(true) {
            return Err("it broke".into());
        }
        if let Some(delay) = input.delay {
            context.note(format!("waiting {delay} s as pid {}", std::process::id()));
            tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
        }
        let mut said = ParrotSaid {
            said: input.text,
            directory: input.directory,
            host_directory: context.host.directory.clone(),
            ask: context.host.ask.clone(),
            pid: std::process::id(),
            ..Default::default()
        };
        if input.ask_model == Some(true) {
            let model = context.host.model_for(Self::WORD)?;
            let schema = json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]});
            let answer =
                host_ext::object_value(model.as_ref(), &schema, "Count.", "How many?", 16, 5.0)
                    .await?;
            said.model_said = answer["n"].as_i64();
        }
        context.deliver(said);
        Ok(())
    }
}
