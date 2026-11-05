//! A worker an app spawns beside its own agent, said without the app's types, so a package can ship one.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// How far a worker may go, in the app's own measure.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerBudget {
    Reading,
    Shell,
}

/// A worker kind that ships as a package.
pub trait GraniteWorkerSpec: 'static {
    /// The word an app's model asks for it by: lowercase, one word.
    const WORD: &'static str;
    /// What it does, said after its word, as a sentence.
    const SUMMARY: &'static str;
    /// The app's tools it keeps, by name; none named keeps the app's reading set.
    fn tool_names() -> Vec<String> {
        Vec::new()
    }
    fn budget() -> WorkerBudget {
        WorkerBudget::Reading
    }
    /// Whether it runs commands.
    fn shell() -> bool {
        false
    }
    /// Who it is, opening its prompt; none keeps the app's own opening.
    fn opening(_word: &str, _project: &str) -> Option<String> {
        None
    }
    /// What its report must say; none keeps the app's own rule.
    fn report_rule() -> Option<String> {
        None
    }
}

type Opening = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

/// A worker spec with its type erased, as a roster keeps it.
#[derive(Clone)]
pub struct AnyWorkerSpec {
    pub word: String,
    pub summary: String,
    pub module: String,
    pub r#type: String,
    pub tool_names: Vec<String>,
    pub budget: WorkerBudget,
    pub shell: bool,
    pub report_rule: Option<String>,
    opening: Opening,
}

impl AnyWorkerSpec {
    pub fn of<W: GraniteWorkerSpec>() -> Self {
        let (module, r#type) = crate::package::names::<W>(None);
        AnyWorkerSpec {
            word: W::WORD.to_owned(),
            summary: W::SUMMARY.to_owned(),
            module,
            r#type,
            tool_names: W::tool_names(),
            budget: W::budget(),
            shell: W::shell(),
            report_rule: W::report_rule(),
            opening: Arc::new(W::opening),
        }
    }

    /// A worker spec described rather than linked: everything but its opening, which is the host's own.
    #[allow(clippy::too_many_arguments)]
    pub fn described(
        word: String,
        summary: String,
        module: String,
        r#type: String,
        tool_names: Vec<String>,
        budget: WorkerBudget,
        shell: bool,
        report_rule: Option<String>,
    ) -> Self {
        AnyWorkerSpec {
            word,
            summary,
            module,
            r#type,
            tool_names,
            budget,
            shell,
            report_rule,
            opening: Arc::new(|_, _| None),
        }
    }

    pub fn opening(&self, word: &str, project: &str) -> Option<String> {
        (self.opening)(word, project)
    }
}
