//! Where a session came from: the session that made it and why, the
//! workspace it was created in, and whether it is kept at all.

use serde::{Deserialize, Deserializer};

use super::{AgentSessionId, AgentSessionOrigin, ConversationItem};

/// Whether a session is written to the history store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentSessionPersistence {
    #[default]
    Durable,
    /// Kept in memory only: never written, never listed, gone when closed
    /// or when the Agent stops.
    Ephemeral,
}

/// What a child session's history starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentChildSessionBase {
    /// Nothing: its first turn is its first context.
    Empty,
    /// The parent's history at its last turn boundary: its head when no
    /// turn runs, and as it stood before the running turn otherwise.
    ParentHead,
}

/// Where a session was created: the workspace's primary root and the Git
/// branch checked out there, each `None` when unknown. The cwd and branch start as creation metadata; an explicit execution binding
/// updates them. A child or a fork takes its source's workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentSessionWorkspace {
    /// An explicitly bound execution root; absent means follow the host.
    pub execution_root: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
}

/// A child session to create under `parent_session_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChildSessionRequest {
    pub parent_session_id: AgentSessionId,
    pub origin: AgentSessionOrigin,
    pub persistence: AgentSessionPersistence,
    pub base: AgentChildSessionBase,
    /// Model-visible context committed with the child's creation, before
    /// its first turn: `ContinuationContext` items only.
    pub context: Vec<ConversationItem>,
    /// What narrows the child further than its parent; the child's scope
    /// is the parent's narrowed by it, so it is never wider. The default
    /// keeps the parent's.
    pub scope: crate::AgentCapabilityScope,
    /// The instructions of the role `scope` names: when it names one, the
    /// child's role instructions are these in place of its parent's;
    /// otherwise the child keeps its parent's role and its instructions.
    pub role_instructions: Option<String>,
}

/// Reads a persisted origin tolerantly: one this build does not know reads
/// as none rather than failing the record.
pub(crate) fn tolerant_origin<'de, D>(
    deserializer: D,
) -> Result<Option<AgentSessionOrigin>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}
