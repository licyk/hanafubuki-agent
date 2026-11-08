//! The memory facts the kernel commits and observers read: a session's
//! contribution state (and why it changed, which is only notified) and the
//! memory citations of a committed answer. They are persisted in the
//! kernel's history, so their serialized form is fixed.
//!
//! Also the memory policy of a sampling step ([`AgentMemoryStep`]), which the
//! `memory` plugin prepares with the step's prompt and the kernel reads.

use std::sync::Arc;

use hanafubuki_agent_foundation::agent_settings::AgentMemoryContributionMode;
use serde::{Deserialize, Serialize};

/// Why a session's memory contribution changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentMemoryContributionCause {
    /// Set from the memory settings as the session was created or forked.
    Initial,
    /// The user turned it on or off.
    User,
    /// A tool brought in external content the session cannot contribute.
    Polluted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryContributionState {
    pub mode: AgentMemoryContributionMode,
    pub revision: u64,
}

/// One memory citation of a committed assistant answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryCitationSnapshot {
    pub generation: u64,
    pub resource_id: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub note: String,
    pub source_session_ids: Vec<String>,
}

/// A final answer with its memory citations taken out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentProcessedMemoryAnswer {
    pub visible_text: String,
    pub citations: Vec<AgentMemoryCitationSnapshot>,
}

/// How memory shapes one sampling step's outcome. The kernel asks it while
/// the step's answers stream and commit and when a tool's output pollutes
/// the session's contribution. Calls are synchronous and bounded.
pub trait AgentMemoryStepPolicy: Send + Sync + 'static {
    /// Whether a tool that brings external content into the session marks
    /// its memory contribution polluted.
    fn excludes_external_context(&self) -> Result<bool, String>;

    /// The byte length of the prefix of a still-streaming answer that is
    /// safe to show; a bounded internal suffix may be withheld until
    /// `process_final_answer` validates it.
    fn visible_stream_prefix_len(&self, text: &str) -> Result<usize, String>;

    /// The final answer's visible text and its validated citations.
    fn process_final_answer(&self, text: &str) -> Result<AgentProcessedMemoryAnswer, String>;
}

/// The memory policy a prompt contributor prepared for a sampling step (the
/// `memory` plugin's), read by the kernel from the step's prepared values. A
/// step without one shows answers as they are and marks no pollution.
#[derive(Clone)]
pub struct AgentMemoryStep {
    pub policy: Arc<dyn AgentMemoryStepPolicy>,
}
