//! `hello`: says hello to a name. The smallest agent package there is, and the installer's fixture.

use std::collections::BTreeMap;

use granite_agents::{AgentContext, AgentPackage, Answer, BoxError};
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct Hello;

#[derive(Deserialize)]
pub struct HelloInput {
    pub name: String,
    pub directory: Option<String>,
}

#[derive(Clone, Default, Serialize)]
pub struct Greeting {
    pub said: String,
}

impl AgentPackage for Hello {
    type Input = HelloInput;
    type Event = String;
    type Output = Answer<Greeting>;
    type Compute = ();

    const WORD: &'static str = "hello";
    const SUMMARY: &'static str = "says hello.";

    fn fields() -> BTreeMap<String, String> {
        BTreeMap::from([("name".to_owned(), "who to greet".to_owned())])
    }

    async fn compute() -> Result<(), BoxError> {
        Ok(())
    }

    fn output(events: Vec<String>) -> Option<Answer<Greeting>> {
        events.last().map(|said| Answer {
            own: Greeting { said: said.clone() },
            ..Default::default()
        })
    }

    async fn run(
        &self,
        input: HelloInput,
        context: AgentContext<(), String>,
    ) -> Result<(), BoxError> {
        context.note(format!("greeting {}", input.name));
        context.deliver(format!("hello, {}", input.name));
        Ok(())
    }
}
