//! The host's side of `granite-agent/1`: one call to an agent in a process of its own. The runner is spawned,
//! handed the call, its notes passed to the host and its asks answered by the host's model, its output taken,
//! and the process reaped, with what it took measured.
//!
//! PIN: ONE PROCESS, ONE CALL, THEN RELEASED. A call that will not end is ended: dropping the call sends
//! `cancel` and SIGTERM to its group, and SIGKILL follows the grace.

mod child;
mod cost;
mod error;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

pub use cost::AgentProcessCost;
pub use error::AgentRemoteError;

use child::AgentChild;
use serde_json::Value;

use crate::{AgentChannel, AgentHost, AgentWire, AskKind, Entry, ModelAnswer, ModelAsk, Perform};

/// Hears what one call's process took, once it is reaped.
pub type CostFn = Box<dyn FnOnce(AgentProcessCost) + Send>;

#[derive(Clone, Debug)]
pub struct AgentProcess {
    pub executable: PathBuf,
    pub environment: BTreeMap<String, String>,
}

/// Ends a call that is dropped before it answered.
struct Ending {
    child: Arc<AgentChild>,
    armed: bool,
}

impl Drop for Ending {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.child.channel.send(&AgentWire::Cancel);
            self.child.terminate(Duration::from_secs(2));
        }
    }
}

impl AgentProcess {
    /// The executable, run with this process's environment and `GRANITE_AGENT=1`.
    pub fn new(executable: impl Into<PathBuf>) -> AgentProcess {
        Self::with_environment(executable, std::env::vars().collect())
    }

    /// The executable, made absolute (a call runs in the host's folder, and a relative path would be read from
    /// there), run with `environment` and `GRANITE_AGENT=1`.
    pub fn with_environment(
        executable: impl Into<PathBuf>,
        mut environment: BTreeMap<String, String>,
    ) -> AgentProcess {
        environment.insert("GRANITE_AGENT".into(), "1".into());
        let executable = executable.into();
        let executable = if executable.is_relative() && executable.components().count() > 1 {
            std::env::current_dir()
                .map(|folder| folder.join(&executable))
                .unwrap_or(executable)
        } else {
            executable
        };
        AgentProcess {
            executable,
            environment,
        }
    }

    fn name(&self) -> String {
        self.executable
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// What the executable carries, as `--describe` says it: nothing is made to say it.
    pub async fn describe(&self) -> Result<Vec<Entry>, AgentRemoteError> {
        let child = AgentChild::spawn(&self.executable, &["--describe"], None, &self.environment)?;
        child.channel.close();
        while let Some(message) = child.channel.recv().await {
            if let AgentWire::Entries(entries) = message {
                child.exited().await;
                return Ok(entries);
            }
        }
        let ended = child.exited().await;
        Err(AgentRemoteError::Ended {
            word: self.name(),
            status: child::describe(ended.status),
            log: child.log(),
        })
    }

    /// One call to `word` in a process of its own, its notes and asks served from `host`; `cost` hears what the
    /// process took once it is reaped. Dropping the future ends the process.
    pub async fn perform(
        &self,
        word: &str,
        input: &[u8],
        host: &AgentHost,
        cost: Option<CostFn>,
    ) -> Result<Vec<u8>, AgentRemoteError> {
        let Ok(json) = serde_json::from_slice::<Value>(input) else {
            return Err(AgentRemoteError::Unreadable {
                word: word.into(),
                reason: "the input is not JSON".into(),
            });
        };
        let directory = host.directory.as_deref().map(Path::new);
        let child = AgentChild::spawn(&self.executable, &[], directory, &self.environment)?;
        let call = Perform {
            word: word.into(),
            input: json,
            directory: host.directory.clone(),
            ask: host.ask.clone(),
            model: host.model.is_some(),
        };
        let asks = Arc::new(AtomicU32::new(0));
        let mut ending = Ending {
            child: child.clone(),
            armed: true,
        };
        let result = serve(&child, word, call, host, &asks).await;
        ending.armed = false;
        // The call is over: its stdin ends, and a process that answered exits at once. One that does not is ended.
        child.channel.close();
        let straggler = {
            let child = child.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                child.terminate(Duration::from_secs(2));
            })
        };
        let ended = child.exited().await;
        straggler.abort();
        if let Some(cost) = cost {
            cost(child.cost(&ended, asks.load(Ordering::SeqCst)));
        }
        match result {
            Ok(bytes) => Ok(bytes),
            Err(AgentRemoteError::Ended { .. }) => Err(AgentRemoteError::Ended {
                word: word.into(),
                status: child::describe(ended.status),
                log: child.log(),
            }),
            Err(error) => Err(error),
        }
    }
}

async fn serve(
    child: &Arc<AgentChild>,
    word: &str,
    call: Perform,
    host: &AgentHost,
    asks: &Arc<AtomicU32>,
) -> Result<Vec<u8>, AgentRemoteError> {
    let mut sent = false;
    let mut standard = false;
    let mut answering = Vec::new();
    let result = loop {
        let Some(message) = child.channel.recv().await else {
            break Err(AgentRemoteError::Ended {
                word: word.into(),
                status: String::new(),
                log: String::new(),
            });
        };
        match message {
            AgentWire::Hello(hello) => {
                let major = hello
                    .wire
                    .strip_prefix("granite-agent/")
                    .and_then(|major| major.parse::<u32>().ok());
                if major != Some(AgentWire::MAJOR) {
                    break Err(AgentRemoteError::Unreadable {
                        word: word.into(),
                        reason: format!(
                            "it speaks {}, and this host {}",
                            hello.wire,
                            AgentWire::VERSION
                        ),
                    });
                }
                if sent {
                    continue;
                }
                sent = true;
                standard = hello.answer.is_some();
                if let Err(error) = child.channel.send(&AgentWire::Perform(call.clone())) {
                    break Err(AgentRemoteError::Unreadable {
                        word: word.into(),
                        reason: format!("the call could not be sent: {error}"),
                    });
                }
            }
            AgentWire::Note(line) => host.note(&line),
            AgentWire::Model(ask) => {
                asks.fetch_add(1, Ordering::SeqCst);
                let (child, host) = (child.clone(), host.clone());
                answering.push(tokio::spawn(async move {
                    let answer = answer(ask, &host).await;
                    let _ = child.channel.send(&AgentWire::Answer(answer));
                }));
            }
            AgentWire::Output(json) => break Ok(serde_json::to_vec(&json).unwrap_or_default()),
            AgentWire::Failed(failure) => {
                // A runner that keeps the answer standard says a refusal as one; anything else it says is a fault.
                break Err(match failure.refusal {
                    Some(refusal) => AgentRemoteError::Refused {
                        word: word.into(),
                        refusal,
                        message: failure.message,
                    },
                    None if standard => AgentRemoteError::Fault {
                        word: word.into(),
                        message: failure.message,
                    },
                    None => AgentRemoteError::Failed {
                        word: word.into(),
                        message: failure.message,
                    },
                });
            }
            other => {
                break Err(AgentRemoteError::Unreadable {
                    word: word.into(),
                    reason: format!("it said `{}` during a call", other.kind()),
                });
            }
        }
    };
    for task in answering {
        task.abort();
    }
    result
}

/// The host's model's answer to one ask, or a sentence saying why there is none.
async fn answer(ask: ModelAsk, host: &AgentHost) -> ModelAnswer {
    let Some(model) = &host.model else {
        return ModelAnswer {
            id: ask.id,
            error: Some("this host has no model loaded".into()),
            ..Default::default()
        };
    };
    match ask.kind {
        AskKind::Object => {
            let schema = ask
                .schema
                .as_ref()
                .map(|schema| serde_json::to_vec(schema).unwrap_or_default())
                .unwrap_or_else(|| b"{}".to_vec());
            match model
                .object(schema, ask.system, ask.user, ask.max_tokens, ask.seconds)
                .await
            {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(object) => ModelAnswer {
                        id: ask.id,
                        object: Some(object),
                        ..Default::default()
                    },
                    Err(_) => ModelAnswer {
                        id: ask.id,
                        error: Some("the model's answer is not JSON".into()),
                        ..Default::default()
                    },
                },
                Err(error) => ModelAnswer {
                    id: ask.id,
                    error: Some(error.to_string()),
                    ..Default::default()
                },
            }
        }
        AskKind::Text => match model
            .text(ask.system, ask.user, ask.max_tokens, ask.seconds)
            .await
        {
            Ok(text) => ModelAnswer {
                id: ask.id,
                text: Some(text),
                ..Default::default()
            },
            Err(error) => ModelAnswer {
                id: ask.id,
                error: Some(error.to_string()),
                ..Default::default()
            },
        },
    }
}
