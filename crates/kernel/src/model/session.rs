use crate::AgentTurnBudgetPolicy;
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionConfig {
    pub provider_id: String,
    pub provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol,
    pub model: String,
    pub instructions: String,
    pub mode: AgentSessionMode,
    pub reasoning_effort: Option<hanafubuki_agent_model_api::ModelReasoningEffort>,
    pub context_window_tokens: Option<u64>,
    pub input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilityPolicy,
    pub auto_compaction_enabled: bool,
    pub auto_compact_threshold_percent: u8,
    pub compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability,
    pub sampling_retry_policy: AgentSamplingRetryPolicy,
    /// Where `context_window_tokens` came from; `None` while it is unknown.
    /// A session written before sources were kept, or with one this build
    /// does not know, has none, and its window counts as configured
    /// ([`Self::context_window_origin`]). Left out when `None`, so such a
    /// session's record is unchanged.
    #[serde(
        default,
        deserialize_with = "tolerant_context_window_source",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_window_source: Option<hanafubuki_agent_model_api::ModelMetadataSource>,
    /// A session written before the strategy existed, or with one this
    /// build does not know, compacts with `standard`.
    #[serde(default, deserialize_with = "tolerant_compaction_strategy")]
    pub compaction_strategy: AgentCompactionStrategy,
    /// What the session's role narrows; empty for the `default` role, and
    /// for a session written before roles existed. Left out when empty, so
    /// such a session's record is unchanged.
    #[serde(default, skip_serializing_if = "super::AgentCapabilityScope::is_empty")]
    pub scope: super::AgentCapabilityScope,
    /// The role's merged instructions, which every request appends to
    /// `instructions` (the base instructions): kept apart so a model
    /// selection, which sets `instructions` again, keeps them.
    #[serde(
        default,
        deserialize_with = "tolerant_role_instructions",
        skip_serializing_if = "Option::is_none"
    )]
    pub role_instructions: Option<String>,
    /// What the person configured for the model, which beats what else is
    /// known of it; the window here is the one a session is given as
    /// configured. Taken again each time the session is bound to its
    /// Provider. Left out when empty, so a session without any keeps its
    /// record unchanged.
    #[serde(
        default,
        deserialize_with = "super::configured_facts::tolerant_configured_facts",
        skip_serializing_if = "super::AgentConfiguredModelFacts::is_empty"
    )]
    pub configured_facts: super::AgentConfiguredModelFacts,
}

/// What starts a reduction of a session's context. Every strategy keeps the
/// standard compaction as its floor (`/compact`, `new_context`, the retry
/// after a provider's context error).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompactionStrategy {
    #[default]
    Standard,
    Background,
    Hierarchical,
}

fn tolerant_compaction_strategy<'de, D>(
    deserializer: D,
) -> Result<AgentCompactionStrategy, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn tolerant_role_instructions<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| value.as_str().map(str::to_string)))
}

fn tolerant_context_window_source<'de, D>(
    deserializer: D,
) -> Result<Option<hanafubuki_agent_model_api::ModelMetadataSource>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

/// The share of the context window at which a session with the `background`
/// strategy starts a compaction in the background, from
/// `[compaction.background]`: the facade puts it in each turn's extensions.
/// A turn without it starts none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentBackgroundCompactionThreshold {
    pub percent: u8,
}

/// The session's compaction in the background, while it has one: running, or
/// ready to be installed at the turn's next checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackgroundCompactionState {
    Running,
    Ready,
}

impl AgentSessionConfig {
    /// The instructions a request carries: the base instructions, then the
    /// role's.
    pub(crate) fn effective_instructions(&self) -> String {
        match &self.role_instructions {
            Some(role) if !self.instructions.is_empty() => {
                format!("{}\n\n{}\n", self.instructions.trim_end(), role.trim())
            }
            _ => self.instructions.clone(),
        }
    }

    /// The configured effort as a request carries it before it is narrowed
    /// to what the route takes (`AgentService::request_reasoning`).
    pub(crate) fn effective_reasoning(
        &self,
    ) -> Option<hanafubuki_agent_model_api::ModelReasoningConfig> {
        self.reasoning_effort.clone().map(|effort| {
            hanafubuki_agent_model_api::ModelReasoningConfig {
                effort,
                max_output_tokens: None,
            }
        })
    }

    /// Where the session's window came from: none while it has none, and
    /// configured for a window without a source (a session written before
    /// sources were kept), which is then never replaced.
    pub(crate) fn context_window_origin(
        &self,
    ) -> Option<hanafubuki_agent_model_api::ModelMetadataSource> {
        self.context_window_tokens.map(|_| {
            self.context_window_source
                .unwrap_or(hanafubuki_agent_model_api::ModelMetadataSource::Configured)
        })
    }

    /// Keeps a window given as configured (without a source: a host's, or
    /// one a session was written with before its configured facts were
    /// kept) as the configured fact, unless the facts name one.
    pub(crate) fn adopt_configured_window(&mut self) {
        if self.configured_facts.context_window_tokens.is_none()
            && self.context_window_origin()
                == Some(hanafubuki_agent_model_api::ModelMetadataSource::Configured)
        {
            self.configured_facts.context_window_tokens = self.context_window_tokens;
        }
    }

    /// Takes `window` (none: unknown) and its source.
    pub(crate) fn set_context_window(
        &mut self,
        window: Option<hanafubuki_agent_model_api::Sourced<u64>>,
    ) {
        self.context_window_tokens = window.as_ref().map(|window| window.value);
        self.context_window_source = window.map(|window| window.source);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSessionModelConfigurationUpdate {
    pub provider_id: String,
    pub provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol,
    pub model: String,
    pub instructions: String,
    pub reasoning_effort: Option<hanafubuki_agent_model_api::ModelReasoningEffort>,
    /// What the person configured for the model, its window included.
    pub configured_facts: super::AgentConfiguredModelFacts,
    pub input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilityPolicy,
    pub auto_compaction_enabled: bool,
    pub auto_compact_threshold_percent: u8,
    pub compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability,
    pub turn_budget: AgentTurnBudgetPolicy,
}

pub const AGENT_SESSION_TITLE_MAX_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionTitleSource {
    Automatic,
    User,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionTitle {
    pub value: String,
    pub source: AgentSessionTitleSource,
}

impl AgentSessionTitle {
    pub fn automatic(value: impl Into<String>) -> Result<Self, String> {
        Self::new(value.into(), AgentSessionTitleSource::Automatic)
    }

    pub fn user(value: impl Into<String>) -> Result<Self, String> {
        Self::new(value.into(), AgentSessionTitleSource::User)
    }

    fn new(value: String, source: AgentSessionTitleSource) -> Result<Self, String> {
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let character_count = value.chars().count();
        if character_count == 0 {
            return Err("Agent conversation title cannot be empty".to_string());
        }
        if character_count > AGENT_SESSION_TITLE_MAX_CHARS {
            return Err(format!(
                "Agent conversation title exceeds {AGENT_SESSION_TITLE_MAX_CHARS} characters"
            ));
        }
        if value.chars().any(char::is_control) {
            return Err("Agent conversation title contains a control character".to_string());
        }
        Ok(Self { value, source })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionTitleRenameRequest {
    pub session_id: AgentSessionId,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskPlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

/// A plan step, as `update_plan` takes it from the model and as the task
/// plan snapshot keeps it. Read tolerantly: an unknown field is ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPlanStep {
    pub step: String,
    pub status: AgentTaskPlanStepStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPlanSnapshot {
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub revision: u64,
    pub explanation: Option<String>,
    pub plan: Vec<AgentTaskPlanStep>,
    pub updated_at: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUserInputOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<AgentUserInputOption>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUserInputRequest {
    pub request_id: AgentUserInputRequestId,
    pub call_id: String,
    pub questions: Vec<AgentUserInputQuestion>,
    pub requested_at: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUserInputAnswer {
    pub question_id: String,
    pub option_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUserInputResolution {
    pub request_id: AgentUserInputRequestId,
    pub answers: Vec<AgentUserInputAnswer>,
    pub resolved_at: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUserInputAccepted {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub request_id: AgentUserInputRequestId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSkillRef {
    pub skill: hanafubuki_agent_skills::SkillId,
}

/// The most images one user message or steering message carries. The
/// application refuses more where it receives them, before it stages any.
pub const MAXIMUM_USER_IMAGES_PER_MESSAGE: usize = 8;

/// The longest host context a steering message carries, in bytes. The host
/// cuts a longer one to it; admission and recovery refuse one past it.
pub const MAXIMUM_HOST_CONTEXT_BYTES: usize = 4 * 1024;

/// What a host submits beside a steering message, written to the world state
/// as a section of `kind` when it changed. The kernel knows no host kind: the
/// section is shown by the renderer the host's plugin registers for it, and
/// left out of a request while none is registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHostContext {
    pub kind: hanafubuki_agent_contracts::ContextSectionKind,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnInput {
    pub text: String,
    #[serde(default)]
    pub images: Vec<super::AgentImageAttachmentRef>,
    #[serde(default)]
    pub skills: Vec<AgentSkillRef>,
    /// What the turn admission contributors produced for this turn
    /// (`collect_turn_extensions`), frozen on the turn. It is neither accepted
    /// from IPC nor persisted with the session.
    #[serde(skip)]
    pub turn: hanafubuki_agent_contracts::TurnExtensions,
    /// Rust-owned world state captured immediately before this turn. It is not
    /// accepted from IPC and canonical history stores only the derived diff.
    #[serde(skip)]
    pub world_state: super::AgentWorldStateSnapshot,
    #[serde(default)]
    pub permission_mode: hanafubuki_agent_approval::AgentPermissionMode,
    /// Who asked for the turn, told to the observers with its admission.
    /// Set by the application, never accepted from IPC nor persisted.
    #[serde(skip)]
    pub origin: hanafubuki_agent_contracts::AgentTurnOrigin,
    /// The automatic title the admission commits if the session has none,
    /// in the same commit as the turn. Set by the application, never
    /// accepted from IPC; only the session's title is persisted.
    #[serde(skip)]
    pub provisional_title: Option<String>,
}

/// Exact one-shot admission request for implementing the currently committed
/// plan. The service atomically switches the session to Default mode and
/// commits the implementation turn under the same session lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPlanImplementationRequest {
    pub session_id: AgentSessionId,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub message_id: AgentMessageId,
}

impl From<String> for AgentTurnInput {
    fn from(text: String) -> Self {
        Self {
            text,
            images: Vec::new(),
            skills: Vec::new(),
            turn: hanafubuki_agent_contracts::TurnExtensions::default(),
            world_state: super::AgentWorldStateSnapshot::default(),
            permission_mode: hanafubuki_agent_approval::AgentPermissionMode::Manual,
            origin: hanafubuki_agent_contracts::AgentTurnOrigin::Programmatic,
            provisional_title: None,
        }
    }
}

impl From<&str> for AgentTurnInput {
    fn from(text: &str) -> Self {
        Self::from(text.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionRuntimeSnapshot {
    pub id: AgentSessionId,
    pub forked_from_session_id: Option<AgentSessionId>,
    #[serde(default)]
    pub parent_session_id: Option<AgentSessionId>,
    #[serde(default, deserialize_with = "super::tolerant_origin")]
    pub origin: Option<super::AgentSessionOrigin>,
    /// Kept in memory only, never in the history store.
    #[serde(default)]
    pub ephemeral: bool,
    pub title: Option<AgentSessionTitle>,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub provider_id: String,
    pub provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol,
    pub model: String,
    pub mode: AgentSessionMode,
    /// The session's role; none for `default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// What the role narrows; none for `default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<super::AgentCapabilityScopeSummary>,
    pub memory_contribution: AgentMemoryContributionState,
    pub reasoning_effort: Option<hanafubuki_agent_model_api::ModelReasoningEffort>,
    pub input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilities,
    pub lifecycle: AgentSessionLifecycle,
    pub usage: Option<AgentModelUsage>,
    pub context_window: AgentContextWindowSnapshot,
    pub compaction: Option<AgentCompactionSnapshot>,
    /// Never persisted: a session read from the history store has none.
    #[serde(default)]
    pub background_compaction: Option<AgentBackgroundCompactionState>,
    /// What the hierarchical compression folded; none for a session that
    /// never ran it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<super::AgentCompressionSummary>,
    pub task_plan: Option<AgentTaskPlanSnapshot>,
    pub pending_user_input: Option<AgentUserInputRequest>,
    pub turn: Option<AgentTurnSnapshot>,
    pub active_activities: Vec<AgentActivitySnapshot>,
    pub recent_activities: Vec<AgentActivitySnapshot>,
    pub next_event_sequence: AgentEventSequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionSummary {
    pub session_id: AgentSessionId,
    pub forked_from_session_id: Option<AgentSessionId>,
    /// The session this one is a child of, and why it was made. A summary
    /// the index kept from before these fields existed reads them as none.
    #[serde(default)]
    pub parent_session_id: Option<AgentSessionId>,
    #[serde(default, deserialize_with = "super::tolerant_origin")]
    pub origin: Option<super::AgentSessionOrigin>,
    /// The workspace's primary root, and the Git branch checked out there,
    /// when the session was created.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub provider_id: String,
    pub provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol,
    pub model: String,
    pub mode: AgentSessionMode,
    /// The session's role; none for `default`, and in a summary kept from
    /// before roles existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub reasoning_effort: Option<hanafubuki_agent_model_api::ModelReasoningEffort>,
    pub title: Option<AgentSessionTitle>,
    pub lifecycle: AgentSessionLifecycle,
    pub usage: Option<AgentModelUsage>,
    pub compaction: Option<AgentCompactionSnapshot>,
    pub latest_turn_phase: Option<AgentTurnPhase>,
    pub created_at: u128,
    pub updated_at: u128,
    pub recovery_marker: Option<crate::AgentTurnRecovery>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionForkRequest {
    pub source_session_id: AgentSessionId,
    pub source_generation: AgentGeneration,
    pub source_revision: AgentRevision,
    pub before_turn_id: AgentTurnId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionForkOutcome {
    pub source_session_id: AgentSessionId,
    pub before_turn_id: AgentTurnId,
    pub restored_prompt: String,
    pub restored_images: Vec<AgentImageAttachmentRef>,
    pub session: AgentSessionRuntimeSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionHeadForkRequest {
    pub source_session_id: AgentSessionId,
    pub source_generation: AgentGeneration,
    pub source_revision: AgentRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionHeadForkOutcome {
    pub source_session_id: AgentSessionId,
    pub source_revision: AgentRevision,
    pub session: AgentSessionRuntimeSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedAgentSessionDeletion {
    pub session_id: AgentSessionId,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
}

/// A deletion prepared for a session and the descendants deleted with it,
/// each held at the generation and revision it had when prepared. The
/// session itself is first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedAgentSessionDeletion {
    pub(crate) sessions: Vec<ConfirmedAgentSessionDeletion>,
}
