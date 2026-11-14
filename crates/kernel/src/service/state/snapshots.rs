use crate::conversation::PersistedAgentRuntimeState;
use crate::model::{
    AgentCompressionSummary, AgentEventSequence, AgentRevision, AgentSessionId,
    AgentSessionRuntimeSnapshot, AgentSessionSummary, AgentTurnSnapshot,
};
use crate::service::SessionRecord;
use hanafubuki_agent_model_api::ModelImageInputCapability;

pub(in crate::service) fn snapshot(
    session_id: AgentSessionId,
    record: &SessionRecord,
) -> AgentSessionRuntimeSnapshot {
    let active_activities = record.runtime.activities.active_snapshots();
    let recent_activities = record.runtime.activities.recent_snapshots();
    AgentSessionRuntimeSnapshot {
        id: session_id,
        forked_from_session_id: record.forked_from_session_id,
        parent_session_id: record.lineage.parent_session_id,
        origin: record.lineage.origin,
        ephemeral: record.lineage.ephemeral,
        title: record.settings.title.clone(),
        generation: record.status.generation,
        revision: AgentRevision::new(record.status.revision),
        provider_id: record.settings.config.provider_id.clone(),
        provider_protocol: record.settings.config.provider_protocol,
        model: record.settings.config.model.clone(),
        mode: record.settings.config.mode,
        role: record.settings.config.scope.role.clone(),
        scope: record.settings.config.scope.summary(),
        memory_contribution: record.settings.memory_contribution,
        reasoning_effort: record.settings.config.reasoning_effort.clone(),
        input_capabilities: record.settings.effective_input_capabilities,
        lifecycle: record.status.lifecycle,
        usage: record.runtime.usage,
        context_window: record.runtime.context_window,
        compaction: record
            .runtime
            .compaction
            .as_ref()
            .map(|compaction| compaction.snapshot.clone()),
        background_compaction: record.runtime.background_compaction.state(),
        compression: record
            .runtime
            .compression
            .as_deref()
            .map(AgentCompressionSummary::of),
        task_plan: record.runtime.task_plan.clone(),
        pending_user_input: record.runtime.pending_user_input.clone(),
        turn: record.runtime.turn.as_ref().map(|turn| AgentTurnSnapshot {
            id: turn.id,
            generation: turn.generation,
            phase: turn.phase,
            sampling_step: turn.sampling_step,
            reasoning: turn.reasoning.snapshot(),
            final_answer_message_id: turn.final_answer_message_id,
            last_error: turn.last_error.clone(),
            terminal_reason: turn.terminal_reason,
            created_at: turn.created_at,
            updated_at: turn.updated_at,
            recovery: turn.recovery,
            pending_steering_count: turn.pending_steering.len(),
            last_consumed_steering_sequence: turn.last_consumed_steering_sequence,
            model_session_generation: turn.model_session_generation,
            permission_mode: turn.permission_mode,
        }),
        active_activities,
        recent_activities,
        next_event_sequence: AgentEventSequence::new(record.journal.next_event_sequence),
        pending_wake_events: record.runtime.wake_queue.len(),
        wake_policy: record.runtime.wake_policy,
    }
}

pub(in crate::service) fn persisted_runtime_snapshot(
    runtime: PersistedAgentRuntimeState,
) -> AgentSessionRuntimeSnapshot {
    let active_activities = runtime
        .activities
        .iter()
        .filter(|activity| !activity.phase.is_terminal())
        .cloned()
        .collect();
    let mut recent_activities = runtime
        .activities
        .iter()
        .filter(|activity| activity.phase.is_terminal())
        .rev()
        .take(32)
        .cloned()
        .collect::<Vec<_>>();
    recent_activities.reverse();
    AgentSessionRuntimeSnapshot {
        id: runtime.id,
        forked_from_session_id: runtime.forked_from_session_id,
        parent_session_id: runtime.parent_session_id,
        origin: runtime.origin,
        ephemeral: false,
        title: runtime.title,
        generation: runtime.generation,
        revision: runtime.revision,
        provider_id: runtime.config.provider_id.clone(),
        provider_protocol: runtime.config.provider_protocol,
        model: runtime.config.model.clone(),
        mode: runtime.config.mode,
        role: runtime.config.scope.role.clone(),
        scope: runtime.config.scope.summary(),
        memory_contribution: runtime.memory_contribution,
        reasoning_effort: runtime.config.reasoning_effort.clone(),
        input_capabilities: runtime
            .config
            .input_capabilities
            .effective(ModelImageInputCapability::Unknown),
        lifecycle: runtime.lifecycle,
        usage: runtime.usage,
        context_window: runtime.context_window,
        compaction: runtime.compaction,
        background_compaction: None,
        compression: runtime
            .compression
            .as_ref()
            .map(AgentCompressionSummary::of),
        task_plan: runtime.task_plan,
        pending_user_input: runtime.pending_user_input,
        turn: runtime.turn.map(|turn| AgentTurnSnapshot {
            id: turn.id,
            generation: turn.generation,
            phase: turn.phase,
            sampling_step: turn.sampling_step,
            reasoning: turn.reasoning.snapshot(),
            final_answer_message_id: turn.final_answer_message_id,
            last_error: turn.last_error,
            terminal_reason: turn.terminal_reason,
            created_at: turn.created_at,
            updated_at: turn.updated_at,
            recovery: turn.recovery,
            pending_steering_count: turn.pending_steering.len(),
            last_consumed_steering_sequence: turn.last_consumed_steering_sequence,
            model_session_generation: turn.model_session_generation,
            permission_mode: turn.permission_mode,
        }),
        pending_wake_events: runtime.wake_queue.len(),
        wake_policy: Default::default(),
        active_activities,
        recent_activities,
        next_event_sequence: AgentEventSequence::new(1),
    }
}

pub(in crate::service) fn session_summary(
    session_id: AgentSessionId,
    record: &SessionRecord,
) -> AgentSessionSummary {
    AgentSessionSummary {
        session_id,
        forked_from_session_id: record.forked_from_session_id,
        parent_session_id: record.lineage.parent_session_id,
        origin: record.lineage.origin,
        cwd: record.lineage.workspace.cwd.clone(),
        branch: record.lineage.workspace.branch.clone(),
        generation: record.status.generation,
        revision: AgentRevision::new(record.status.revision),
        provider_id: record.settings.config.provider_id.clone(),
        provider_protocol: record.settings.config.provider_protocol,
        model: record.settings.config.model.clone(),
        mode: record.settings.config.mode,
        role: record.settings.config.scope.role.clone(),
        reasoning_effort: record.settings.config.reasoning_effort.clone(),
        title: record.settings.title.clone(),
        lifecycle: record.status.lifecycle,
        usage: record.runtime.usage,
        compaction: record
            .runtime
            .compaction
            .as_ref()
            .map(|compaction| compaction.snapshot.clone()),
        latest_turn_phase: record.runtime.turn.as_ref().map(|turn| turn.phase),
        created_at: record.created_at,
        updated_at: record.status.updated_at,
        recovery_marker: record.runtime.turn.as_ref().and_then(|turn| turn.recovery),
    }
}
