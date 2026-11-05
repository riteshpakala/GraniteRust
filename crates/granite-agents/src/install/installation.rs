use serde::{Deserialize, Serialize};

use super::AgentSource;
use crate::Entry;

/// One installed runner, as `installation.json` keeps it beside the executable: where it came from, what it was
/// built with, the executable's digest, and what it carries. A host offers its agents from this alone.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstallation {
    pub format: String,
    /// The package's name, and the folder's.
    pub name: String,
    pub source: AgentSource,
    /// The source's commit, when it is a repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// The granite-agents the runner was built against: its version, and its commit when it came from git.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granite: Option<String>,
    /// The executable, relative to the installation's folder.
    pub executable: String,
    /// The executable's SHA-256, lowercase hex.
    pub sha256: String,
    pub size: u64,
    /// The executable's modification time when installed, seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<f64>,
    /// ISO 8601.
    pub built_at: String,
    pub entries: Vec<Entry>,
}

impl AgentInstallation {
    pub const FORMAT: &'static str = "granite-agent-installation/1";
    pub const FILE: &'static str = "installation.json";

    /// The words of the agents it carries.
    pub fn words(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| entry.word.clone())
            .collect()
    }
}

/// Now, as ISO 8601 in UTC to the second: `2026-11-05T18:00:00Z`.
pub fn now_iso8601() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}
