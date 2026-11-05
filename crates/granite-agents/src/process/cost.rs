use serde::{Deserialize, Serialize};

/// What one call's process took, measured by the host when it reaped it: the measure a scheduler places a call
/// by, and what a person reads when an agent was slow or large.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProcessCost {
    /// From the spawn to the exit.
    pub seconds: f64,
    /// User and system time.
    pub cpu_seconds: f64,
    /// The most memory it held at once.
    pub peak_bytes: i64,
    /// How it ended: `exit 0`, `exit 1`, `signal 9`.
    pub status: String,
    /// How many times it asked the host's model; a cost recorded before asks were counted reads as none.
    #[serde(default)]
    pub asks: u32,
}

impl AgentProcessCost {
    /// "released · peak 412 MB · 1.8 s CPU in 2.4 s"
    pub fn sentence(&self) -> String {
        let megabytes = self.peak_bytes as f64 / 1_048_576.0;
        let peak = if megabytes >= 1024.0 {
            format!("{:.1} GB", megabytes / 1024.0)
        } else {
            format!("{megabytes:.0} MB")
        };
        format!(
            "released · peak {peak} · {:.1} s CPU in {:.1} s",
            self.cpu_seconds, self.seconds
        )
    }
}
