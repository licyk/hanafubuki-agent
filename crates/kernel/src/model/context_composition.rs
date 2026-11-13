//! What a model request contained, group by group, and what a session's
//! `/context` reads of it ([`AgentService::session_context`]).
//!
//! [`AgentService::session_context`]: crate::AgentService::session_context

use serde::{Deserialize, Serialize};

use super::AgentModelUsage;

/// One model request's contents in estimated tokens, group by group: the
/// Agent's one estimate (about four bytes a token of the request as it is
/// serialized, and its rule for each image), applied to each group's part.
/// The groups add up to the request's estimate, the figure every check of a
/// request against the window counts with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextComposition {
    /// The base instructions, with what plan mode and the compression add.
    pub instructions_tokens: u64,
    /// The tools offered, by the provider that planned them, in the order
    /// the request lists them.
    pub tools: Vec<AgentContextTools>,
    /// Each context section by its kind, in the order the request first
    /// shows it.
    pub sections: Vec<AgentContextSection>,
    pub user_messages_tokens: u64,
    pub assistant_messages_tokens: u64,
    /// The reasoning sent back to the provider.
    pub reasoning_tokens: u64,
    /// Tool calls and their outputs, `tool_search`'s included.
    pub tool_calls_tokens: u64,
    pub continuation_tokens: u64,
    /// Compaction summaries, and a provider's own compacted context.
    pub compaction_summaries_tokens: u64,
    pub images: u64,
    pub image_tokens: u64,
    /// The rest of the request: the model, the tool choice, the reasoning
    /// settings and the serialized form's own framing. It is zero when the
    /// groups' rounding (each rounds up) exceeds it, which takes very many
    /// small groups.
    pub other_tokens: u64,
    /// The request's estimate.
    pub total_tokens: u64,
}

/// The tools one provider planned for a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextTools {
    /// The tools' provider (`AgentPlannedTool::provider_id`); `None` for
    /// the tool search a provider runs itself, which no provider planned.
    pub provider_id: Option<String>,
    pub count: u64,
    pub tokens: u64,
}

/// The context sections of one kind in a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextSection {
    pub kind: String,
    pub tokens: u64,
}

/// What a session's `/context` shows: the request it reads, the window and
/// the usage whose cumulative cache figures give the session's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionContext {
    pub request: AgentContextRequest,
    /// The session's window, else the one its model's facts resolve, as
    /// `/status` shows it.
    pub context_window_tokens: Option<u64>,
    pub usage: Option<AgentModelUsage>,
}

/// The request `/context` reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum AgentContextRequest {
    /// The last request a sampling step of this run sent.
    Sent(AgentSentRequest),
    /// No request was sent in this run since the history last changed shape
    /// (a compaction): an estimate of the next, built outside a sampling
    /// step. Nothing a plugin prepares for a step is in it, and outside a
    /// turn no context section is.
    Next(AgentContextComposition),
    /// Nor could the next request be built, for this reason.
    Unavailable(String),
}

/// A request as a sampling step sent it, kept for the session while the
/// Agent runs, never persisted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSentRequest {
    pub composition: AgentContextComposition,
    pub captured_at_ms: u64,
    /// What the provider reported of the request once its response came
    /// back; `None` before, or when it reported no usage.
    pub reported: Option<AgentReportedPrompt>,
}

/// A provider's count of one request's prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReportedPrompt {
    /// The prompt total: uncached input + cache read + cache write.
    pub input_tokens: u64,
    /// `None` when the provider reported no such figure.
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}
