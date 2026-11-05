//! What a runner carries, or a host offers: package agents and worker specs, one per word.
//!
//! PIN: LISTED IN THE RUNNER, FOUND BY THE HOST. A Swift package places a record in a Mach-O section; a Rust
//! runner lists its packages in its `main` (`roster![…]`), and a host still finds them by `--describe` and the
//! installation record, never by reading the binary.

use crate::{AgentPackage, AnyAgentPackage, AnyWorkerSpec, Entry, GraniteWorkerSpec};

#[derive(Clone)]
pub enum RosterEntry {
    Package(AnyAgentPackage),
    Worker(AnyWorkerSpec),
}

impl RosterEntry {
    pub fn word(&self) -> &str {
        match self {
            RosterEntry::Package(package) => &package.word,
            RosterEntry::Worker(worker) => &worker.word,
        }
    }

    pub fn module(&self) -> &str {
        match self {
            RosterEntry::Package(package) => &package.module,
            RosterEntry::Worker(worker) => &worker.module,
        }
    }
}

#[derive(Clone, Default)]
pub struct Roster {
    pub entries: Vec<RosterEntry>,
}

impl Roster {
    /// The entries, the first of a word kept.
    pub fn new(entries: impl IntoIterator<Item = RosterEntry>) -> Roster {
        let mut kept: Vec<RosterEntry> = Vec::new();
        for entry in entries {
            if !kept.iter().any(|seen| seen.word() == entry.word()) {
                kept.push(entry);
            }
        }
        Roster { entries: kept }
    }

    pub fn empty() -> Roster {
        Roster::default()
    }

    pub fn with_package<P: AgentPackage>(self) -> Roster {
        self.with(RosterEntry::Package(AnyAgentPackage::of::<P>()))
    }

    pub fn with_worker<W: GraniteWorkerSpec>(self) -> Roster {
        self.with(RosterEntry::Worker(AnyWorkerSpec::of::<W>()))
    }

    pub fn with(mut self, entry: RosterEntry) -> Roster {
        if !self.entries.iter().any(|seen| seen.word() == entry.word()) {
            self.entries.push(entry);
        }
        self
    }

    pub fn packages(&self) -> Vec<&AnyAgentPackage> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                RosterEntry::Package(package) => Some(package),
                RosterEntry::Worker(_) => None,
            })
            .collect()
    }

    pub fn workers(&self) -> Vec<&AnyWorkerSpec> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                RosterEntry::Worker(worker) => Some(worker),
                RosterEntry::Package(_) => None,
            })
            .collect()
    }

    pub fn package(&self, word: &str) -> Option<&AnyAgentPackage> {
        self.packages()
            .into_iter()
            .find(|package| package.word == word)
    }

    /// What `--describe` prints.
    pub fn describe(&self) -> Vec<Entry> {
        self.entries.iter().map(Entry::of).collect()
    }
}
