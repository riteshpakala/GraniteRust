#![allow(dead_code)]
//! Packages and a host model the tests share.

use std::collections::BTreeMap;
use std::sync::Mutex;

use granite_agents::{
    AgentAnswer, AgentContext, AgentHostError, AgentHostModel, AgentMake, AgentPackage,
    AgentRefusal, Answer, BoxError, BoxFuture, RefusalKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Says its text back, `times` times; waits first when told.
#[derive(Default)]
pub struct Echo;

#[derive(Deserialize)]
pub struct EchoInput {
    pub text: String,
    pub times: Option<u32>,
    pub delay: Option<f64>,
    pub directory: Option<String>,
}

#[derive(Clone, Default, Serialize)]
pub struct Said {
    pub said: String,
}

impl AgentPackage for Echo {
    type Input = EchoInput;
    type Event = String;
    type Output = Answer<Said>;
    type Compute = ();

    const WORD: &'static str = "echo";
    const SUMMARY: &'static str = "says its text back.";

    fn fields() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("text".into(), "what to say".into()),
            ("times".into(), "how many times; once when left out".into()),
        ])
    }

    async fn compute() -> Result<(), BoxError> {
        Ok(())
    }

    fn output(events: Vec<String>) -> Option<Answer<Said>> {
        (!events.is_empty()).then(|| Answer {
            own: Said {
                said: events.join(" "),
            },
            ..Default::default()
        })
    }

    async fn run(
        &self,
        input: EchoInput,
        context: AgentContext<(), String>,
    ) -> Result<(), BoxError> {
        if let Some(delay) = input.delay {
            tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
        }
        for _ in 0..input.times.unwrap_or(1) {
            context.deliver(input.text.clone());
        }
        Ok(())
    }
}

/// Makes a card, and answers it in each way the standard can be broken (`mode`).
#[derive(Default)]
pub struct Maker;

#[derive(Deserialize)]
pub struct MakerInput {
    pub name: String,
    pub mode: String,
    pub directory: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(transparent)]
pub struct Loose(pub Value);
impl AgentAnswer for Loose {}

impl AgentPackage for Maker {
    type Input = MakerInput;
    type Event = Value;
    type Output = Loose;
    type Compute = ();

    const WORD: &'static str = "maker";
    const SUMMARY: &'static str = "makes a card.";

    fn makes() -> Vec<AgentMake> {
        vec![AgentMake::new(
            "card",
            "a card",
            &["cards/*"],
            Some("a chart"),
        )]
    }

    async fn compute() -> Result<(), BoxError> {
        Ok(())
    }

    fn output(events: Vec<Value>) -> Option<Loose> {
        events.last().cloned().map(Loose)
    }

    async fn run(
        &self,
        input: MakerInput,
        context: AgentContext<(), Value>,
    ) -> Result<(), BoxError> {
        if input.mode == "refuse" {
            return Err(Box::new(AgentRefusal::one(
                RefusalKind::Call,
                "`name` is empty: give the card a name",
            )));
        }
        let folder = std::path::PathBuf::from(input.directory.clone().unwrap_or_default());
        std::fs::create_dir_all(folder.join("cards"))?;
        let path = format!("cards/{}.card.json", input.name);
        let schema = if input.mode == "payload" {
            "other-card/1"
        } else {
            "test-card/1"
        };
        std::fs::write(folder.join(&path), json!({"schema": schema}).to_string())?;
        let mut answer = json!({
            "path": path, "paths": [path],
            "made": [{"kind": "card", "name": input.name, "path": path, "state": "written"}],
            "expected": [{"kind": "card", "name": input.name}],
            "payloads": [{"path": path, "schema": "test-card/1"}],
            "card": input.name,
        });
        match input.mode.as_str() {
            "path" => answer["path"] = json!("cards/elsewhere.card.json"),
            "kind" => answer["made"][0]["kind"] = json!("chart"),
            "stamp" => answer["format"] = json!("granite-answer/1"),
            _ => {}
        }
        context.deliver(answer);
        Ok(())
    }
}

/// Reports what its host offered, and asks the model when it has one.
#[derive(Default)]
pub struct Hosted;

#[derive(Deserialize)]
pub struct HostedInput {
    pub ask_model: Option<bool>,
}

#[derive(Clone, Default, Serialize)]
pub struct Offered {
    pub directory: Option<String>,
    pub ask: Option<String>,
    pub n: Option<i64>,
}

impl AgentPackage for Hosted {
    type Input = HostedInput;
    type Event = Offered;
    type Output = Answer<Offered>;
    type Compute = ();

    const WORD: &'static str = "hosted";
    const SUMMARY: &'static str = "says what its host offered.";

    async fn compute() -> Result<(), BoxError> {
        Ok(())
    }

    fn output(events: Vec<Offered>) -> Option<Answer<Offered>> {
        events.last().map(|offered| Answer {
            own: offered.clone(),
            ..Default::default()
        })
    }

    async fn run(
        &self,
        input: HostedInput,
        context: AgentContext<(), Offered>,
    ) -> Result<(), BoxError> {
        let mut offered = Offered {
            directory: context.host.directory.clone(),
            ask: context.host.ask.clone(),
            n: None,
        };
        if input.ask_model == Some(true) {
            let model = context.host.model_for(Self::WORD)?;
            let answer = granite_agents::host_ext::object_value(
                model.as_ref(),
                &json!({"type": "object"}),
                "Count.",
                "How many?",
                16,
                5.0,
            )
            .await?;
            offered.n = answer["n"].as_i64();
        }
        context.note("offered");
        context.deliver(offered);
        Ok(())
    }
}

/// A host model that answers each object ask with the next of its answers.
pub struct FakeModel {
    pub answers: Mutex<Vec<Value>>,
    pub asked: Mutex<Vec<String>>,
}

impl FakeModel {
    pub fn new(answers: Vec<Value>) -> std::sync::Arc<FakeModel> {
        std::sync::Arc::new(FakeModel {
            answers: Mutex::new(answers),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl AgentHostModel for FakeModel {
    fn object(
        &self,
        _schema: Vec<u8>,
        _system: String,
        user: String,
        _max_tokens: u32,
        _seconds: f64,
    ) -> BoxFuture<'_, Result<Vec<u8>, AgentHostError>> {
        self.asked.lock().unwrap().push(user);
        let next = {
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                None
            } else {
                Some(answers.remove(0))
            }
        };
        Box::pin(async move {
            next.map(|value| serde_json::to_vec(&value).unwrap())
                .ok_or_else(|| AgentHostError::NoAnswer("nothing left to say".into()))
        })
    }

    fn text(
        &self,
        _system: String,
        _user: String,
        _max_tokens: u32,
        _seconds: f64,
    ) -> BoxFuture<'_, Result<String, AgentHostError>> {
        Box::pin(async { Ok("text".into()) })
    }
}
