use serde::{Deserialize, Serialize};

/// What an agent makes, one kind: what it is, in the words a reading can match to a request, where it writes
/// (relative to the host's folder, `*` for one path component, `Folder/*` for everything under it), and what it
/// never does. An answer's `made` and `expected` kinds are these kinds.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct AgentMake {
    pub kind: String,
    pub what: String,
    pub into: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub never: Option<String>,
}

impl AgentMake {
    pub fn new(kind: &str, what: &str, into: &[&str], never: Option<&str>) -> Self {
        AgentMake {
            kind: kind.to_owned(),
            what: what.to_owned(),
            into: into.iter().map(|path| (*path).to_owned()).collect(),
            never: never.map(str::to_owned),
        }
    }
}
