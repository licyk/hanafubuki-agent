//! What a read of a configuration file had to fix, kept for the surfaces
//! that report it (`doctor`, a host's recovery notice).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// A configuration file the Agent could not use as it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfigRecovery {
    pub file: PathBuf,
    pub problem: AgentConfigProblem,
    /// Why the original could not be kept in `.recovery/` and replaced, when
    /// it could not: the fixed values are then used from memory, and the
    /// file refuses writes until the Agent restarts.
    pub read_only: Option<String>,
    pub recorded_at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentConfigProblem {
    /// Not a TOML document the Agent could read; the defaults are used.
    Unusable { reason: String },
    /// Values the Agent could not use, each replaced as described.
    Repaired { repairs: Vec<String> },
}

impl AgentConfigRecovery {
    pub(crate) fn new(
        file: PathBuf,
        problem: AgentConfigProblem,
        read_only: Option<String>,
    ) -> Self {
        let recorded_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        Self {
            file,
            problem,
            read_only,
            recorded_at_unix_ms,
        }
    }
}
