use super::*;

pub(in crate::service) fn validate_history_append(
    record: &SessionRecord,
    items: &[ConversationItem],
    limits: AgentHardLimits,
) -> Result<(), AgentError> {
    let item_count = record
        .conversation
        .history
        .len()
        .saturating_add(items.len());
    enforce_size("history items", item_count, limits.maximum_history_items)?;
    let appended_bytes = items.iter().fold(0usize, |total, item| {
        total.saturating_add(item.retained_bytes())
    });
    let history_bytes = record
        .conversation
        .history_bytes
        .saturating_add(appended_bytes);
    enforce_size("history bytes", history_bytes, limits.maximum_history_bytes)?;
    let mut combined = record
        .conversation
        .history
        .iter()
        .map(|item| &item.content)
        .collect::<Vec<_>>();
    combined.extend(items.iter());
    validate_turn_boundary_refs(&combined)?;
    Ok(())
}

pub(in crate::service) fn validate_persisted_history(
    history: &[ConversationItem],
    config: &AgentSessionConfig,
    limits: AgentHardLimits,
) -> Result<(), AgentError> {
    enforce_size("history items", history.len(), limits.maximum_history_items)?;
    validate_turn_boundaries(history)?;
    validate_skill_context_groups(history)?;
    validate_world_state_history(history)?;
    let history_bytes = history.iter().try_fold(0usize, |total, item| {
        match item {
            ConversationItem::CanonicalInitialContext { digest, content } => {
                enforce_size(
                    "canonical initial context bytes",
                    content.len(),
                    limits.maximum_history_bytes,
                )?;
                let mut hasher = Sha256::new();
                hasher.update(config.instructions.as_bytes());
                hasher.update(content.as_bytes());
                if digest != &<[u8; 32]>::from(hasher.finalize()) {
                    return Err(AgentError::Persistence(
                        "canonical initial context digest does not match its content".to_string(),
                    ));
                }
            }
            ConversationItem::ToolCall {
                call_id,
                namespace,
                name,
                ..
            } => {
                validate_bounded_identifier(call_id)?;
                validate_bounded_identifier(name)?;
                if let Some(namespace) = namespace {
                    validate_bounded_identifier(namespace)?;
                }
            }
            ConversationItem::ToolOutput { call_id, .. } => {
                validate_bounded_identifier(call_id)?;
            }
            ConversationItem::AssistantMessage {
                text,
                model_text,
                proposed_plan,
                ..
            } => match model_text {
                Some(canonical) => {
                    if canonical == text {
                        return Err(AgentError::Persistence(
                            "assistant model text must be omitted when it equals visible text"
                                .to_string(),
                        ));
                    }
                    let mut projection =
                        crate::service::plan_output::PlanOutputProjection::default();
                    projection.push_str(canonical);
                    projection.finish();
                    if projection.visible_text() != text
                        || projection.proposed_plan()
                            != proposed_plan.as_ref().map(|plan| plan.text.as_str())
                    {
                        return Err(AgentError::Persistence(
                            "assistant plan projection does not match canonical model text"
                                .to_string(),
                        ));
                    }
                }
                None if proposed_plan.is_some() => {
                    return Err(AgentError::Persistence(
                        "assistant plan requires distinct canonical model text".to_string(),
                    ));
                }
                None => {}
            },
            ConversationItem::PermissionReview {
                review_id,
                call_id,
                action_summary,
                status,
                assessment,
                completed_at,
                ..
            } => {
                validate_bounded_identifier(review_id)?;
                validate_bounded_identifier(call_id)?;
                enforce_size("permission review summary", action_summary.len(), 2 * 1024)?;
                validate_permission_review_state(*status, assessment.as_ref(), *completed_at)?;
            }
            ConversationItem::TurnStart { .. }
            | ConversationItem::WorldStateUpdate { .. }
            | ConversationItem::SkillContext { .. }
            | ConversationItem::UserMessage { .. }
            | ConversationItem::SteeringMessage { .. }
            | ConversationItem::ToolSelection { .. }
            | ConversationItem::ContinuationContext { .. }
            | ConversationItem::CompactionSummary { .. }
            | ConversationItem::ProviderCompaction { .. } => {}
        }
        Ok::<_, AgentError>(total.saturating_add(item.retained_bytes()))
    })?;
    enforce_size("history bytes", history_bytes, limits.maximum_history_bytes)?;
    Ok(())
}

fn validate_world_state_history(history: &[ConversationItem]) -> Result<(), AgentError> {
    let mut state = BTreeMap::new();
    for item in history {
        let ConversationItem::WorldStateUpdate { updates } = item else {
            continue;
        };
        // One update replaces every section of the new state and removes
        // every one of the old, so it can name up to twice the sections.
        if updates.is_empty() || updates.len() > 2 * MAXIMUM_AGENT_WORLD_STATE_SECTIONS {
            return Err(AgentError::Persistence(
                "Agent history contains an invalid world-state update count".to_string(),
            ));
        }
        let mut kinds = BTreeSet::new();
        for update in updates {
            let kind = match update {
                AgentWorldStateUpdate::Replace {
                    kind,
                    version,
                    content,
                } => {
                    if *kind == ContextSectionKind::TOOL_CATALOG_OMISSIONS
                        || content.trim().is_empty()
                        || content.len() > MAXIMUM_AGENT_WORLD_STATE_SECTION_BYTES
                        || AgentWorldStateSection::new(kind.clone(), content.clone()).version
                            != *version
                    {
                        return Err(AgentError::Persistence(
                            "Agent history contains an invalid world-state replacement".to_string(),
                        ));
                    }
                    state.insert(kind.clone(), *version);
                    kind
                }
                AgentWorldStateUpdate::Remove {
                    kind,
                    previous_version,
                } => {
                    if state.get(kind) != Some(previous_version) {
                        return Err(AgentError::Persistence(
                            "Agent history contains a stale world-state removal".to_string(),
                        ));
                    }
                    state.remove(kind);
                    kind
                }
            };
            if !kinds.insert(kind) {
                return Err(AgentError::Persistence(
                    "Agent history contains duplicate world-state section updates".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::service) fn validate_permission_review_state(
    status: PermissionReviewStatus,
    assessment: Option<&PermissionAssessment>,
    completed_at: Option<u64>,
) -> Result<(), AgentError> {
    let valid = match status {
        PermissionReviewStatus::Reviewing => assessment.is_none() && completed_at.is_none(),
        PermissionReviewStatus::Approved => {
            completed_at.is_some()
                && assessment
                    .is_some_and(|value| value.outcome == PermissionAssessmentOutcome::Allow)
        }
        PermissionReviewStatus::Denied => {
            completed_at.is_some()
                && assessment
                    .is_some_and(|value| value.outcome == PermissionAssessmentOutcome::Deny)
        }
        PermissionReviewStatus::TimedOut
        | PermissionReviewStatus::Failed
        | PermissionReviewStatus::Aborted => assessment.is_none() && completed_at.is_some(),
    };
    if valid {
        Ok(())
    } else {
        Err(AgentError::Protocol(
            "permission review status, assessment, and completion time are inconsistent"
                .to_string(),
        ))
    }
}

pub(in crate::service) fn validate_persisted_message_state(
    session_id: AgentSessionId,
    next_message_id: u64,
    turn: Option<&PersistedAgentTurn>,
    history: &[ConversationItem],
) -> Result<(), AgentError> {
    let mut message_ids = HashSet::new();
    let mut maximum_message_id = 0u64;
    for item in history {
        if let ConversationItem::AssistantMessage { message_id, .. } = item {
            if !message_ids.insert(*message_id) {
                return Err(AgentError::Persistence(format!(
                    "Agent session {} contains duplicate assistant message id {}",
                    session_id, message_id
                )));
            }
            maximum_message_id = maximum_message_id.max(message_id.value());
        }
    }
    if next_message_id == 0 || next_message_id <= maximum_message_id {
        return Err(AgentError::Persistence(format!(
            "Agent session {} has an invalid next assistant message id",
            session_id
        )));
    }

    let Some(turn) = turn else {
        return Ok(());
    };
    turn.reasoning.validate().map_err(|error| {
        AgentError::Persistence(format!(
            "Agent session {} has an invalid reasoning checkpoint: {}",
            session_id, error
        ))
    })?;
    if turn.reasoning.snapshot().sampling_step > turn.sampling_step {
        return Err(AgentError::Persistence(format!(
            "Agent session {} has reasoning from a future sampling step",
            session_id
        )));
    }
    let terminal_reason_valid = match turn.phase {
        AgentTurnPhase::Completed => turn.terminal_reason.is_none(),
        AgentTurnPhase::Cancelled | AgentTurnPhase::Failed | AgentTurnPhase::Interrupted => {
            turn.terminal_reason.is_some()
        }
        AgentTurnPhase::Sampling
        | AgentTurnPhase::RunningTools
        | AgentTurnPhase::WaitingForUserInput => turn.terminal_reason.is_none(),
    };
    if !terminal_reason_valid {
        return Err(AgentError::Persistence(format!(
            "Agent session {} has an inconsistent turn terminal reason",
            session_id
        )));
    }
    match (turn.phase, turn.final_answer_message_id) {
        (AgentTurnPhase::Completed, Some(final_id)) => {
            let turn_start = history
                .iter()
                .rposition(|item| {
                    matches!(item, ConversationItem::TurnStart { turn_id } if *turn_id == turn.id)
                })
                .ok_or_else(|| {
                    AgentError::Persistence(format!(
                        "Agent session {} is missing its latest turn boundary",
                        session_id
                    ))
                })?;
            if !history[turn_start..].iter().any(|item| {
                matches!(
                    item,
                    ConversationItem::AssistantMessage {
                        message_id,
                        phase: Some(AgentMessagePhase::FinalAnswer),
                        ..
                    } if *message_id == final_id
                )
            }) {
                return Err(AgentError::Persistence(format!(
                    "Agent session {} has a dangling final answer message id {}",
                    session_id, final_id
                )));
            }
        }
        (AgentTurnPhase::Completed, None) => {
            return Err(AgentError::Persistence(format!(
                "Agent session {} completed without a final answer message id",
                session_id
            )));
        }
        (_, Some(_)) => {
            return Err(AgentError::Persistence(format!(
                "Agent session {} has a final answer reference on a non-completed turn",
                session_id
            )));
        }
        (_, None) => {}
    }
    Ok(())
}

pub(super) fn validate_turn_boundaries(history: &[ConversationItem]) -> Result<(), AgentError> {
    validate_turn_boundary_refs(&history.iter().collect::<Vec<_>>())
}

fn validate_turn_boundary_refs(history: &[&ConversationItem]) -> Result<(), AgentError> {
    if history.is_empty() {
        return Ok(());
    }
    let first_turn = history
        .iter()
        .position(|item| matches!(**item, ConversationItem::TurnStart { .. }));
    if first_turn != Some(0) {
        let prefix_end = first_turn.unwrap_or(history.len());
        if prefix_end == 0
            || history[..prefix_end].iter().any(|item| {
                !matches!(
                    **item,
                    ConversationItem::CanonicalInitialContext { .. }
                        | ConversationItem::WorldStateUpdate { .. }
                        | ConversationItem::SkillContext { .. }
                        | ConversationItem::UserMessage { .. }
                        | ConversationItem::AssistantMessage { .. }
                        | ConversationItem::SteeringMessage { .. }
                        | ConversationItem::CompactionSummary { .. }
                        | ConversationItem::ProviderCompaction { .. }
                )
            })
            || !history[..prefix_end].iter().any(|item| {
                matches!(
                    **item,
                    ConversationItem::CompactionSummary { .. }
                        | ConversationItem::ProviderCompaction { .. }
                )
            })
        {
            return Err(AgentError::Persistence(
                "Agent history contains an invalid compacted model prefix".to_string(),
            ));
        }
        if prefix_end == history.len() {
            return Ok(());
        }
        return validate_turn_boundary_refs(&history[prefix_end..]);
    }
    let mut turn_ids = HashSet::new();
    let mut has_turn = false;
    let mut awaiting_user = false;
    let mut initial_world_state = false;
    for (index, item) in history.iter().enumerate() {
        match *item {
            ConversationItem::TurnStart { turn_id } => {
                if awaiting_user || !turn_ids.insert(*turn_id) {
                    return Err(AgentError::Persistence(
                        "Agent history contains an invalid or duplicate turn boundary".to_string(),
                    ));
                }
                has_turn = true;
                awaiting_user = true;
                initial_world_state = false;
            }
            ConversationItem::WorldStateUpdate { .. } if has_turn && awaiting_user => {
                if initial_world_state {
                    return Err(AgentError::Persistence(
                        "Agent history contains duplicate initial world-state updates".to_string(),
                    ));
                }
                initial_world_state = true;
            }
            ConversationItem::WorldStateUpdate { .. } if has_turn => {}
            ConversationItem::SkillContext { .. } if has_turn && awaiting_user => {}
            ConversationItem::SkillContext { .. }
                if has_turn && skill_context_follows_tool_output(history, index) => {}
            ConversationItem::UserMessage { .. } if has_turn && awaiting_user => {
                awaiting_user = false;
            }
            ConversationItem::CompactionSummary { .. }
            | ConversationItem::ProviderCompaction { .. }
            | ConversationItem::SkillContext { .. }
            | ConversationItem::UserMessage { .. } => {
                return Err(AgentError::Persistence(
                    "Agent history contains an item outside its turn boundary".to_string(),
                ));
            }
            _ if !has_turn || awaiting_user => {
                return Err(AgentError::Persistence(
                    "Agent history contains an incomplete turn boundary".to_string(),
                ));
            }
            _ => {}
        }
    }
    if awaiting_user {
        return Err(AgentError::Persistence(
            "Agent history ends with an incomplete turn boundary".to_string(),
        ));
    }
    Ok(())
}
