//! A package agent with its type erased, as a roster keeps it: what it is called and takes, and a call that
//! takes JSON and answers JSON.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use super::run::{PackageCall, Thrown};
use super::{AgentPackage, AgentPackageError, PackageHost, PerformFn, names};
use crate::{
    AgentHost, AgentMake, AgentRefusal, AnswerCheck, AnswerStandard, BoxError, Entry,
    GraniteSchema, RefusalKind, json,
};

#[derive(Clone)]
pub struct AnyAgentPackage {
    pub word: String,
    pub summary: String,
    /// The module that declares it.
    pub module: String,
    /// Its type, as `Module.Type`.
    pub r#type: String,
    /// What its input takes.
    pub schema: GraniteSchema,
    /// The lines it says beside its fields.
    pub fields: BTreeMap<String, String>,
    /// How many calls run at once; none: no bound.
    pub lanes: Option<u32>,
    pub guide: String,
    pub makes: Vec<AgentMake>,
    call: PerformFn,
}

impl AnyAgentPackage {
    /// The input field a host fills with the folder it works in, and does not show a model.
    pub const DIRECTORY_FIELD: &'static str = "directory";

    pub fn of<P: AgentPackage>() -> AnyAgentPackage {
        let (module, r#type) = names::<P>(P::module());
        AnyAgentPackage {
            word: P::WORD.to_owned(),
            summary: P::SUMMARY.to_owned(),
            module,
            r#type,
            schema: P::schema(),
            fields: P::fields(),
            lanes: P::lanes(),
            guide: P::guide(),
            makes: P::makes(),
            call: Arc::new(|input, host| Box::pin(perform::<P>(input, host))),
        }
    }

    /// A package described rather than linked: what an entry says of it, and a call that runs wherever it lives.
    pub fn described(entry: &Entry, perform: PerformFn) -> AnyAgentPackage {
        AnyAgentPackage {
            word: entry.word.clone(),
            summary: entry.summary.clone(),
            module: entry.module.clone(),
            r#type: entry.r#type.clone(),
            schema: entry
                .schema
                .clone()
                .unwrap_or(GraniteSchema::Object(Vec::new())),
            fields: entry.fields.clone().unwrap_or_default(),
            lanes: entry.lanes,
            guide: entry.guide.clone().unwrap_or_default(),
            makes: entry.makes.clone().unwrap_or_default(),
            call: perform,
        }
    }

    /// Whether the package takes the host's folder.
    pub fn takes_directory(&self) -> bool {
        self.schema
            .fields()
            .iter()
            .any(|field| field.name == Self::DIRECTORY_FIELD)
    }

    /// One call: the input decoded, the run hosted on the package's own center and offered `host`, the answer
    /// checked, stamped and encoded. Dropping the future cancels the run.
    pub async fn perform(&self, input: &[u8], host: AgentHost) -> Result<Vec<u8>, BoxError> {
        (self.call)(input.to_vec(), host).await
    }
}

fn call_id(word: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0) as u64;
    let mixed = (nanos ^ (std::process::id() as u64) << 16).wrapping_add(
        NEXT.fetch_add(1, Ordering::SeqCst)
            .wrapping_mul(0x9E37_79B9),
    );
    format!("{word}-{:08x}", mixed & 0xffff_ffff)
}

async fn perform<P: AgentPackage>(bytes: Vec<u8>, offered: AgentHost) -> Result<Vec<u8>, BoxError> {
    let word = P::WORD;
    let input: P::Input =
        serde_json::from_slice(&bytes).map_err(|error| AgentPackageError::Input {
            word: word.into(),
            reason: error.to_string(),
        })?;
    let host = PackageHost::<P>::shared();
    let id = call_id(word);
    // The tap before the run, so no event is missed; it ends with the run.
    let tap = host.agent.events(&id);
    let heard = tokio::spawn(tap.collect());
    let thrown = Thrown::default();
    let call = PackageCall {
        id,
        input,
        host: offered.clone(),
        thrown: thrown.clone(),
    };
    if let Err(error) = host.agent.perform(call).await {
        heard.abort();
        // What the run returned, not the engine's sentence about it: a refusal keeps its kind.
        if let Some(original) = thrown.take() {
            let said = original.to_string();
            return Err(match AgentRefusal::of(original.as_ref()) {
                Some(refusal) => Box::new(AgentPackageError::Refused {
                    word: word.into(),
                    refusal,
                    said,
                }),
                None => original,
            });
        }
        return Err(Box::new(match error {
            granite::AgentRunError::Cancelled | granite::AgentRunError::Superseded(_) => {
                AgentPackageError::Cancelled
            }
            other => AgentPackageError::Fault(other.to_string()),
        }));
    }
    let events = heard.await.unwrap_or_default();
    let output =
        P::output(events).ok_or_else(|| AgentPackageError::NoOutput { word: word.into() })?;
    let refused = |refusal: AgentRefusal, said: String| AgentPackageError::Refused {
        word: word.into(),
        refusal,
        said,
    };
    let Ok(Value::Object(object)) = serde_json::to_value(&output) else {
        let refusal = AgentRefusal::one(
            RefusalKind::Answer,
            format!("{word} answered something that is not an object"),
        );
        return Err(Box::new(refused(refusal.clone(), refusal.to_string())));
    };
    let problems = AnswerCheck::problems(&object, word, &P::makes(), offered.directory.as_deref());
    if !problems.is_empty() {
        let refusal = AgentRefusal::new(RefusalKind::Answer, problems);
        let said = format!(
            "{word}'s answer breaks {}: {refusal}",
            AnswerStandard::FORMAT
        );
        return Err(Box::new(refused(refusal, said)));
    }
    Ok(json::sorted(&Value::Object(AnswerCheck::stamped(
        &object, word,
    )))?)
}
