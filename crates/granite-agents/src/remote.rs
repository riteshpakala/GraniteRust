//! Agents that live in a runner, as a roster keeps them: each entry's call is one [`AgentProcess`] of the
//! runner, at most its lanes at once, after `check` passes.
//!
//! PIN: NOTHING RESIDENT. An entry holds a path and a description; a process exists only while a call does.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::process::AgentRemoteError;
use crate::{AgentProcess, BoxError, Entry, RosterEntry};

pub struct RemoteAgent;

/// The tamper check a call passes before its process is started.
pub type Check = Arc<dyn Fn() -> Result<(), AgentRemoteError> + Send + Sync>;

impl RemoteAgent {
    /// The roster entries of `executable`, from what it said of itself. Each call checks `check` first and tells
    /// the host what its process took as its last note.
    pub fn entries(
        described: &[Entry],
        executable: &Path,
        environment: BTreeMap<String, String>,
        check: Check,
    ) -> Vec<RosterEntry> {
        let process = Arc::new(AgentProcess::with_environment(executable, environment));
        described
            .iter()
            .map(|entry| {
                let lanes = Arc::new(Semaphore::new(entry.lanes.unwrap_or(2).max(1) as usize));
                let (process, check, word) = (process.clone(), check.clone(), entry.word.clone());
                entry.roster_entry(Arc::new(move |input, host| {
                    let (process, check, lanes, word) =
                        (process.clone(), check.clone(), lanes.clone(), word.clone());
                    Box::pin(async move {
                        check().map_err(|error| Box::new(error) as BoxError)?;
                        let _lane = lanes
                            .acquire_owned()
                            .await
                            .map_err(|error| Box::new(error) as BoxError)?;
                        let noting = host.clone();
                        let cost = Box::new(move |cost: crate::AgentProcessCost| {
                            noting.note(&cost.sentence())
                        });
                        process
                            .perform(&word, &input, &host, Some(cost))
                            .await
                            .map_err(|error| Box::new(error) as BoxError)
                    })
                }))
            })
            .collect()
    }
}
