//! One agent, described: everything a host needs to offer it and call it without the type that declares it.
//! What `--describe` prints and what an installation keeps.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::package::PerformFn;
use crate::{AgentMake, AnyAgentPackage, AnyWorkerSpec, GraniteSchema, RosterEntry, WorkerBudget};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Package,
    Worker,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub kind: EntryKind,
    pub word: String,
    pub summary: String,
    pub module: String,
    #[serde(rename = "type")]
    pub r#type: String,
    // A package agent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<GraniteSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lanes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub makes: Option<Vec<AgentMake>>,
    // A worker spec's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_names: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<WorkerBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_rule: Option<String>,
}

impl Entry {
    /// Every field an entry may say, in the order the standard lists them.
    pub const FIELD_NAMES: [&'static str; 14] = [
        "kind",
        "word",
        "summary",
        "module",
        "type",
        "schema",
        "fields",
        "lanes",
        "guide",
        "makes",
        "toolNames",
        "budget",
        "shell",
        "reportRule",
    ];

    /// What a roster entry says of itself.
    pub fn of(entry: &RosterEntry) -> Entry {
        match entry {
            RosterEntry::Package(package) => Entry {
                kind: EntryKind::Package,
                word: package.word.clone(),
                summary: package.summary.clone(),
                module: package.module.clone(),
                r#type: package.r#type.clone(),
                schema: Some(package.schema.clone()),
                fields: Some(package.fields.clone()),
                lanes: package.lanes,
                guide: (!package.guide.is_empty()).then(|| package.guide.clone()),
                makes: (!package.makes.is_empty()).then(|| package.makes.clone()),
                tool_names: None,
                budget: None,
                shell: None,
                report_rule: None,
            },
            RosterEntry::Worker(worker) => Entry {
                kind: EntryKind::Worker,
                word: worker.word.clone(),
                summary: worker.summary.clone(),
                module: worker.module.clone(),
                r#type: worker.r#type.clone(),
                schema: None,
                fields: None,
                lanes: None,
                guide: None,
                makes: None,
                tool_names: Some(worker.tool_names.clone()),
                budget: Some(worker.budget),
                shell: Some(worker.shell),
                report_rule: worker.report_rule.clone(),
            },
        }
    }

    /// The entry a roster keeps, its package's calls made by `perform`.
    pub fn roster_entry(&self, perform: PerformFn) -> RosterEntry {
        match self.kind {
            EntryKind::Package => RosterEntry::Package(AnyAgentPackage::described(self, perform)),
            EntryKind::Worker => RosterEntry::Worker(AnyWorkerSpec::described(
                self.word.clone(),
                self.summary.clone(),
                self.module.clone(),
                self.r#type.clone(),
                self.tool_names.clone().unwrap_or_default(),
                self.budget.unwrap_or(WorkerBudget::Reading),
                self.shell.unwrap_or(false),
                self.report_rule.clone(),
            )),
        }
    }
}
