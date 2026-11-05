//! One call a server answers: `hello`, then the host's `perform` started on the roster's package, the host's
//! answers matched to the run's asks, a `cancel` or the host's leaving handled, and exactly one `output` or
//! `failed` said at the end. Swift Granite's `AgentServerCall`.

use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::Exit;
use super::model::ChannelHostModel;
use crate::{
    AgentChannel, AgentHost, AgentPackageError, AgentRefusal, AgentWire, AnswerStandard, Failure,
    Hello, Perform, Roster,
};

struct CallState {
    run: Option<JoinHandle<()>>,
    model: Option<Arc<ChannelHostModel>>,
    cancelled: bool,
    ended: Option<Exit>,
    done: Option<oneshot::Sender<Exit>>,
}

#[derive(Clone)]
pub(crate) struct ServerCall {
    channel: Arc<dyn AgentChannel>,
    roster: Arc<Roster>,
    state: Arc<Mutex<CallState>>,
}

impl ServerCall {
    pub(crate) fn new(channel: Arc<dyn AgentChannel>, roster: Roster) -> ServerCall {
        ServerCall {
            channel,
            roster: Arc::new(roster),
            state: Arc::new(Mutex::new(CallState {
                run: None,
                model: None,
                cancelled: false,
                ended: None,
                done: None,
            })),
        }
    }

    pub(crate) async fn serve(&self) -> Exit {
        let hello = Hello {
            wire: AgentWire::VERSION.into(),
            pid: Some(std::process::id() as i32),
            answer: Some(AnswerStandard::FORMAT.into()),
        };
        let _ = self.channel.send(&AgentWire::Hello(hello));
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.state.lock().unwrap();
            if let Some(ended) = state.ended {
                return ended;
            }
            state.done = Some(tx);
        }
        let listening = self.clone();
        tokio::spawn(async move { listening.listen().await });
        rx.await.unwrap_or(Exit::Failed)
    }

    /// Ends the call: the run is stopped, and says so; before a run, the call ends as cancelled.
    pub(crate) fn cancel(&self) {
        let run = {
            let mut state = self.state.lock().unwrap();
            state.cancelled = true;
            state.run.take()
        };
        match run {
            Some(run) => {
                run.abort();
                self.finish(
                    Exit::Cancelled,
                    AgentWire::Failed(Failure::said("cancelled")),
                );
            }
            None => self.finish(
                Exit::Cancelled,
                AgentWire::Failed(Failure::said("cancelled before the call began")),
            ),
        }
    }

    async fn listen(&self) {
        while let Some(message) = self.channel.recv().await {
            match message {
                AgentWire::Perform(perform) => self.start(perform),
                AgentWire::Answer(answer) => {
                    let model = self.state.lock().unwrap().model.clone();
                    if let Some(model) = model {
                        model.resolve(answer)
                    }
                }
                AgentWire::Cancel => self.cancel(),
                other => self.finish(
                    Exit::Unreadable,
                    AgentWire::Failed(Failure::said(format!(
                        "the host said `{}`, which is not a host's message",
                        other.kind()
                    ))),
                ),
            }
        }
        // The host sends nothing more: a call it made runs on, but no ask of the model can be answered. A host
        // that closes its end at once (`echo '{"perform": …}' | runner`) is served; only `cancel` or SIGTERM
        // cancels. A host that left before calling ends the process.
        let (model, called) = {
            let state = self.state.lock().unwrap();
            (
                state.model.clone(),
                state.run.is_some() || state.ended.is_some(),
            )
        };
        if let Some(model) = model {
            model.leave();
        }
        if !called {
            self.finish(
                Exit::Unreadable,
                AgentWire::Failed(Failure::said("the host left before making a call")),
            );
        }
    }

    fn start(&self, perform: Perform) {
        {
            let state = self.state.lock().unwrap();
            if state.run.is_some() || state.ended.is_some() {
                drop(state);
                return self.finish(
                    Exit::Unreadable,
                    AgentWire::Failed(Failure::said(
                        "a process answers one call; the host sent a second `perform`",
                    )),
                );
            }
        }
        let Some(package) = self.roster.package(&perform.word).cloned() else {
            let carried: Vec<String> = self
                .roster
                .packages()
                .iter()
                .map(|package| format!("`{}`", package.word))
                .collect();
            let carried = if carried.is_empty() {
                "none".to_owned()
            } else {
                carried.join(", ")
            };
            return self.finish(
                Exit::Failed,
                AgentWire::Failed(Failure::said(format!(
                    "no agent is called `{}` in this process; it carries {carried}",
                    perform.word
                ))),
            );
        };
        let model = perform
            .model
            .then(|| ChannelHostModel::new(self.channel.clone()));
        let noting = self.channel.clone();
        let mut host = AgentHost::none().with_note(move |line| {
            let _ = noting.send(&AgentWire::Note(line.to_owned()));
        });
        host.directory = perform.directory.clone();
        host.ask = perform.ask.clone();
        if let Some(model) = &model {
            host = host.with_model(model.clone());
        }
        let input = serde_json::to_vec(&perform.input).unwrap_or_default();
        let call = self.clone();
        let word = perform.word.clone();
        let mut state = self.state.lock().unwrap();
        state.model = model;
        let task = tokio::spawn(async move {
            match package.perform(&input, host).await {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(json) => call.finish(Exit::Answered, AgentWire::Output(json)),
                    Err(_) => call.finish(
                        Exit::Failed,
                        AgentWire::Failed(Failure::said(format!(
                            "{word} answered something that is not JSON"
                        ))),
                    ),
                },
                Err(error) => {
                    let cancelled = call.state.lock().unwrap().cancelled;
                    if matches!(
                        error.downcast_ref::<AgentPackageError>(),
                        Some(AgentPackageError::Cancelled)
                    ) {
                        return call.finish(
                            Exit::Cancelled,
                            AgentWire::Failed(Failure::said("cancelled")),
                        );
                    }
                    let said = error.to_string();
                    match AgentRefusal::of(error.as_ref()) {
                        Some(refusal) => call.finish(
                            if cancelled {
                                Exit::Cancelled
                            } else {
                                Exit::Refused
                            },
                            AgentWire::Failed(Failure::refused(said, refusal)),
                        ),
                        None => call.finish(
                            if cancelled {
                                Exit::Cancelled
                            } else {
                                Exit::Failed
                            },
                            AgentWire::Failed(Failure::said(said)),
                        ),
                    }
                }
            }
        });
        state.run = Some(task);
        let cancelled = state.cancelled;
        drop(state);
        if cancelled {
            self.cancel();
        }
    }

    /// Says `message` and ends the call with `exit`, once.
    fn finish(&self, exit: Exit, message: AgentWire) {
        let done = {
            let mut state = self.state.lock().unwrap();
            if state.ended.is_some() {
                return;
            }
            state.ended = Some(exit);
            state.done.take()
        };
        let _ = self.channel.send(&message);
        if let Some(done) = done {
            let _ = done.send(exit);
        }
    }
}
