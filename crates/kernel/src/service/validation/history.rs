use super::*;

pub(in crate::service) fn validate_persisted_compaction(
    compaction: Option<&AgentCompactionSnapshot>,
    session_revision: AgentRevision,
) -> Result<(), AgentError> {
    let Some(compaction) = compaction else {
        return Ok(());
    };
    if compaction.source_revision.value() > session_revision.value()
        || compaction.updated_at < compaction.created_at
        || compaction.target_window.previous_window_id != Some(compaction.source_window_id)
        || compaction.target_window.validate().is_err()
    {
        return Err(AgentError::Persistence(
            "Agent compaction metadata is inconsistent with the session".to_string(),
        ));
    }
    if compaction.trigger == AgentCompactionTrigger::Automatic
        && compaction.turn_generation.is_none()
    {
        return Err(AgentError::Persistence(
            "automatic Agent compaction is missing its turn generation".to_string(),
        ));
    }
    Ok(())
}

/// Compares the window identity and chain position that a checkpoint match must
/// preserve. `prefill_input_tokens` is usage accounting that the live window
/// backfills after the checkpoint snapshot is captured, and
/// `new_context_requested` is a transient request bit, so neither participates
/// in structural checkpoint consistency.
fn window_identity_matches(a: AgentContextWindowSnapshot, b: AgentContextWindowSnapshot) -> bool {
    a.window_number == b.window_number
        && a.first_window_id == b.first_window_id
        && a.previous_window_id == b.previous_window_id
        && a.window_id == b.window_id
}

pub(in crate::service) fn validate_context_window_checkpoint(
    window: AgentContextWindowSnapshot,
    compaction: Option<&AgentCompactionSnapshot>,
    boundary: &AgentModelContextBoundary,
    session_revision: AgentRevision,
    limits: AgentHardLimits,
) -> Result<(), AgentError> {
    window.validate()?;
    match boundary {
        AgentModelContextBoundary::Full => {
            if window.window_number != 0 {
                return Err(AgentError::Persistence(
                    "a non-initial Agent context window is missing its checkpoint".to_string(),
                ));
            }
        }
        AgentModelContextBoundary::Compacted { checkpoint } => {
            if checkpoint.snapshot.status != AgentCompactionStatus::Completed
                || compaction.is_some_and(|current| {
                    current.status == AgentCompactionStatus::Completed
                        && current != &checkpoint.snapshot
                })
                || !window_identity_matches(checkpoint.snapshot.target_window, window)
                || checkpoint.snapshot.source_revision.value() > session_revision.value()
                || checkpoint.replacement_history.is_empty()
                || checkpoint.replacement_history.len() > limits.maximum_history_items
                || checkpoint.window_chain.len() < 2
                || checkpoint.window_chain.len() > limits.maximum_history_items
                || !checkpoint
                    .window_chain
                    .last()
                    .is_some_and(|last| window_identity_matches(*last, window))
                || checkpoint.initial_context_digest == [0; 32]
                || checkpoint
                    .replacement_history
                    .iter()
                    .any(|item| item.item_id.value() > checkpoint.future_items_after_item_id)
            {
                return Err(AgentError::Persistence(
                    "the Agent compaction checkpoint is inconsistent".to_string(),
                ));
            }
            for (index, current) in checkpoint.window_chain.iter().copied().enumerate() {
                current.validate()?;
                if current.window_number != u64::try_from(index).unwrap_or(u64::MAX)
                    || current.first_window_id != checkpoint.window_chain[0].window_id
                    || (index > 0
                        && current.previous_window_id
                            != Some(checkpoint.window_chain[index - 1].window_id))
                {
                    return Err(AgentError::Persistence(
                        "the Agent context window checkpoint chain is discontinuous".to_string(),
                    ));
                }
            }
            if checkpoint.snapshot.source_window_id
                != checkpoint.window_chain[checkpoint.window_chain.len() - 2].window_id
            {
                return Err(AgentError::Persistence(
                    "the Agent compaction source window is not the checkpoint predecessor"
                        .to_string(),
                ));
            }
            if let Some(marker_digest) =
                checkpoint
                    .replacement_history
                    .iter()
                    .find_map(|item| match &item.content {
                        ConversationItem::CanonicalInitialContext { digest, .. } => Some(*digest),
                        _ => None,
                    })
            {
                if marker_digest != checkpoint.initial_context_digest {
                    return Err(AgentError::Persistence(
                        "the Agent initial-context checkpoint digest does not match its marker"
                            .to_string(),
                    ));
                }
            }
            let mut ids = HashSet::new();
            if checkpoint
                .replacement_history
                .iter()
                .any(|item| !ids.insert(item.item_id))
            {
                return Err(AgentError::Persistence(
                    "the Agent compaction checkpoint contains duplicate item IDs".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::service) fn validate_persisted_usage(
    usage: Option<AgentModelUsage>,
    config: &AgentSessionConfig,
) -> Result<(), AgentError> {
    let Some(usage) = usage else {
        return Ok(());
    };
    if usage.context_window_tokens != config.context_window_tokens {
        return Err(AgentError::Persistence(
            "Agent usage context window does not match the session config".to_string(),
        ));
    }
    if usage.cumulative_input_tokens < usage.last_response_input_tokens
        || usage.cumulative_output_tokens < usage.last_response_output_tokens
    {
        return Err(AgentError::Persistence(
            "Agent cumulative usage is smaller than the last response usage".to_string(),
        ));
    }
    usage.context_consumed_tokens.ok_or_else(|| {
        AgentError::Persistence("Agent context consumption is missing".to_string())
    })?;
    Ok(())
}

pub(in crate::service) fn validate_user_input_answers(
    request: &AgentUserInputRequest,
    answers: &[AgentUserInputAnswer],
) -> Result<(), AgentError> {
    if answers.len() != request.questions.len() {
        return Err(AgentError::StaleUserInput);
    }
    let mut answered = HashSet::new();
    for answer in answers {
        if !answered.insert(answer.question_id.as_str()) {
            return Err(AgentError::StaleUserInput);
        }
        let question = request
            .questions
            .iter()
            .find(|question| question.id == answer.question_id)
            .ok_or(AgentError::StaleUserInput)?;
        if !question
            .options
            .iter()
            .any(|option| option.id == answer.option_id)
        {
            return Err(AgentError::StaleUserInput);
        }
    }
    Ok(())
}
