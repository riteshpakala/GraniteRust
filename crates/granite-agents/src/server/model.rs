//! The host's model, as an agent in another process asks it: each ask a `model` message under an id, each answer
//! the host's `answer` under the same id.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::oneshot;

use crate::{
    AgentChannel, AgentHostError, AgentHostModel, AgentWire, AskKind, BoxFuture, ModelAnswer,
    ModelAsk,
};

pub(crate) struct ChannelHostModel {
    channel: Arc<dyn AgentChannel>,
    next: AtomicU64,
    waiting: Mutex<(HashMap<u64, oneshot::Sender<ModelAnswer>>, bool)>,
}

struct Waiting<'a> {
    model: &'a ChannelHostModel,
    id: u64,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.model.waiting.lock().unwrap().0.remove(&self.id);
    }
}

impl ChannelHostModel {
    pub(crate) fn new(channel: Arc<dyn AgentChannel>) -> Arc<ChannelHostModel> {
        Arc::new(ChannelHostModel {
            channel,
            next: AtomicU64::new(0),
            waiting: Mutex::new((HashMap::new(), false)),
        })
    }

    /// The host's answer to an ask.
    pub(crate) fn resolve(&self, answer: ModelAnswer) {
        if let Some(waiter) = self.waiting.lock().unwrap().0.remove(&answer.id) {
            let _ = waiter.send(answer);
        }
    }

    /// The host is gone: every ask waiting, and every ask after, ends without an answer.
    pub(crate) fn leave(&self) {
        let mut waiting = self.waiting.lock().unwrap();
        waiting.1 = true;
        waiting.0.clear();
    }

    async fn ask(
        &self,
        kind: AskKind,
        schema: Option<Value>,
        system: String,
        user: String,
        max_tokens: u32,
        seconds: f64,
    ) -> Result<ModelAnswer, AgentHostError> {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let rx = {
            let mut waiting = self.waiting.lock().unwrap();
            if waiting.1 {
                return Err(AgentHostError::NoAnswer("the host is gone".into()));
            }
            let (tx, rx) = oneshot::channel();
            waiting.0.insert(id, tx);
            rx
        };
        let _waiting = Waiting { model: self, id };
        let ask = ModelAsk {
            id,
            kind,
            schema,
            system,
            user,
            max_tokens,
            seconds,
        };
        if let Err(error) = self.channel.send(&AgentWire::Model(ask)) {
            return Err(AgentHostError::NoAnswer(format!(
                "the ask could not be sent: {error}"
            )));
        }
        rx.await
            .map_err(|_| AgentHostError::NoAnswer("the host is gone".into()))
    }
}

impl AgentHostModel for ChannelHostModel {
    fn object(
        &self,
        schema: Vec<u8>,
        system: String,
        user: String,
        max_tokens: u32,
        seconds: f64,
    ) -> BoxFuture<'_, Result<Vec<u8>, AgentHostError>> {
        Box::pin(async move {
            let schema = serde_json::from_slice(&schema).ok();
            let answer = self
                .ask(AskKind::Object, schema, system, user, max_tokens, seconds)
                .await?;
            if let Some(error) = answer.error {
                return Err(AgentHostError::NoAnswer(error));
            }
            let Some(object) = answer.object else {
                return Err(AgentHostError::NoAnswer(
                    "the host answered no object".into(),
                ));
            };
            serde_json::to_vec(&object).map_err(|error| AgentHostError::Answer(error.to_string()))
        })
    }

    fn text(
        &self,
        system: String,
        user: String,
        max_tokens: u32,
        seconds: f64,
    ) -> BoxFuture<'_, Result<String, AgentHostError>> {
        Box::pin(async move {
            let answer = self
                .ask(AskKind::Text, None, system, user, max_tokens, seconds)
                .await?;
            if let Some(error) = answer.error {
                return Err(AgentHostError::NoAnswer(error));
            }
            Ok(answer.text.unwrap_or_default())
        })
    }
}
