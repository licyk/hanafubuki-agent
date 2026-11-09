//! The live state of one session, grouped by what each part is for.
//!
//! The identity (id, lineage, creation time) never changes. Every other group
//! is replaced as a whole when a staged durable mutation is rolled back, so a
//! new field only has to join a group to be covered by rollback.

use super::*;

#[derive(Clone)]
pub(in crate::service) struct SessionRecord {
    pub(in crate::service) session_id: AgentSessionId,
    pub(in crate::service) forked_from_session_id: Option<AgentSessionId>,
    pub(in crate::service) lineage: SessionLineage,
    pub(in crate::service) created_at: u128,
    pub(in crate::service) settings: SessionSettings,
    pub(in crate::service) status: SessionStatus,
    pub(in crate::service) conversation: SessionConversation,
    pub(in crate::service) journal: SessionJournal,
    pub(in crate::service) runtime: SessionRuntime,
}

/// Where the session came from and whether it is kept: its parent and why
/// it was made, the workspace it was created in, and whether it lives in
/// memory only.
#[derive(Clone, Default)]
pub(in crate::service) struct SessionLineage {
    pub(in crate::service) parent_session_id: Option<AgentSessionId>,
    pub(in crate::service) origin: Option<crate::AgentSessionOrigin>,
    pub(in crate::service) workspace: crate::AgentSessionWorkspace,
    pub(in crate::service) ephemeral: bool,
}

/// How the session samples: its model, configuration and policies.
#[derive(Clone)]
pub(in crate::service) struct SessionSettings {
    pub(in crate::service) model: Arc<dyn ModelPort>,
    pub(in crate::service) title: Option<AgentSessionTitle>,
    pub(in crate::service) goal: Option<crate::AgentGoal>,
    pub(in crate::service) config: AgentSessionConfig,
    pub(in crate::service) memory_contribution: AgentMemoryContributionState,
    pub(in crate::service) effective_input_capabilities: ModelInputCapabilities,
    pub(in crate::service) budget_policy: AgentTurnBudgetPolicy,
}

/// Where the session stands: lifecycle, the generation and revision that
/// stale requests are checked against, and the commit in flight.
#[derive(Clone)]
pub(in crate::service) struct SessionStatus {
    pub(in crate::service) lifecycle: AgentSessionLifecycle,
    pub(in crate::service) generation: AgentGeneration,
    pub(in crate::service) revision: u64,
    pub(in crate::service) updated_at: u128,
    pub(in crate::service) disposing: bool,
    pub(in crate::service) persisting_revision: Option<u64>,
}

/// The canonical history the model sees, and the mutations not yet durable.
#[derive(Clone)]
pub(in crate::service) struct SessionConversation {
    pub(in crate::service) history: Arc<Vec<Arc<AgentHistoryItem>>>,
    pub(in crate::service) history_version: u64,
    pub(in crate::service) pending_history_mutations: Vec<AgentHistoryMutation>,
    pub(in crate::service) next_history_item_id: u64,
    pub(in crate::service) model_context_boundary: AgentModelContextBoundary,
    pub(in crate::service) history_base: Option<AgentHistoryBase>,
    pub(in crate::service) history_bytes: usize,
    pub(in crate::service) next_message_id: u64,
}

/// The bounded UI event journal.
#[derive(Clone)]
pub(in crate::service) struct SessionJournal {
    pub(in crate::service) events: VecDeque<AgentEvent>,
    pub(in crate::service) next_event_sequence: u64,
}

/// The running work: the turn, a compaction, and what they produced.
#[derive(Clone)]
pub(in crate::service) struct SessionRuntime {
    pub(in crate::service) turn: Option<TurnRecord>,
    pub(in crate::service) compaction: Option<CompactionRecord>,
    pub(in crate::service) usage: Option<AgentModelUsage>,
    pub(in crate::service) context_window: AgentContextWindowSnapshot,
    pub(in crate::service) activities: AgentActivityTracker,
    pub(in crate::service) task_plan: Option<AgentTaskPlanSnapshot>,
    pub(in crate::service) pending_user_input: Option<AgentUserInputRequest>,
    /// What the user allowed for the rest of the session
    /// (`ApprovalUserDecision::AllowForSession`). Never persisted: a session
    /// recovered by a new Agent instance starts without any.
    pub(in crate::service) allowances: BTreeSet<crate::service::interaction::SessionAllowance>,
    /// The permission mode the user chose for the session's next turns, in
    /// place of the configured one. Never persisted, like `allowances`.
    pub(in crate::service) permission_mode: Option<hanafubuki_agent_approval::AgentPermissionMode>,
    /// The sandbox mode the user chose for the session's commands, in
    /// place of the configured one. Never persisted, like the permission
    /// mode; the kernel only keeps it for the application.
    pub(in crate::service) sandbox_mode:
        Option<hanafubuki_agent_foundation::agent_settings::AgentSandboxMode>,
    /// The compaction running in the background, never persisted (a restart
    /// drops it) and not an active operation: turns and every other change
    /// go on while it runs.
    pub(in crate::service) background_compaction:
        crate::service::compaction::BackgroundCompactionSlot,
    /// The hierarchical compression state, persisted as a chain of links in
    /// the runtime checkpoints that change it. Shared by `Arc`, so a rollback
    /// state never copies it.
    pub(in crate::service) compression: Option<Arc<crate::AgentCompressionState>>,
    /// The chain's last link this run staged. A rollback restores it with
    /// the rest of the runtime, so it is always the last durable link once
    /// the commit settles; none until this run writes one.
    pub(in crate::service) compression_head: Option<crate::model::CompressionChainHead>,
    /// `search_context`'s cache of document features, never persisted.
    pub(in crate::service) compression_search: crate::service::compression::SearchCache,
    /// The content store and the interrupt the nudge still has to book,
    /// kept apart from the checkpoints.
    pub(in crate::service) compression_runtime: crate::service::compression::CompressionRuntime,
}

/// Bounded rollback state for one staged durable mutation: every group but
/// the identity. Canonical history is shared by `Arc`, so taking it never
/// clones historical message or tool-output payloads.
#[derive(Clone)]
pub(in crate::service) struct SessionRollbackState {
    settings: SessionSettings,
    status: SessionStatus,
    conversation: SessionConversation,
    journal: SessionJournal,
    runtime: SessionRuntime,
}

impl SessionRecord {
    pub(in crate::service) fn rollback_state(&self) -> SessionRollbackState {
        SessionRollbackState {
            settings: self.settings.clone(),
            status: self.status.clone(),
            conversation: self.conversation.clone(),
            journal: self.journal.clone(),
            runtime: self.runtime.clone(),
        }
    }

    pub(in crate::service) fn restore_rollback_state(&mut self, state: SessionRollbackState) {
        self.settings = state.settings;
        self.status = state.status;
        self.conversation = state.conversation;
        self.journal = state.journal;
        self.runtime = state.runtime;
    }
}

impl SessionStatus {
    /// A session that was just created and has no revision yet.
    pub(in crate::service) fn created(now: u128) -> Self {
        Self {
            lifecycle: AgentSessionLifecycle::Available,
            generation: AgentGeneration::new(0),
            revision: 0,
            updated_at: now,
            disposing: false,
            persisting_revision: None,
        }
    }
}

impl SessionJournal {
    pub(in crate::service) fn empty() -> Self {
        Self {
            events: VecDeque::new(),
            next_event_sequence: 1,
        }
    }
}

impl SessionRuntime {
    /// Nothing running and nothing produced.
    pub(in crate::service) fn idle() -> Self {
        Self {
            turn: None,
            compaction: None,
            usage: None,
            context_window: AgentContextWindowSnapshot::initial(),
            activities: AgentActivityTracker::default(),
            task_plan: None,
            pending_user_input: None,
            allowances: BTreeSet::new(),
            permission_mode: None,
            sandbox_mode: None,
            background_compaction: Default::default(),
            compression: None,
            compression_head: None,
            compression_search: Default::default(),
            compression_runtime: Default::default(),
        }
    }
}
