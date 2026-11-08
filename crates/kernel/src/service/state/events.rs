use super::*;

pub(in crate::service) fn record_event(
    record: &mut SessionRecord,
    identity: TurnIdentity,
    kind: AgentEventKind,
    limits: AgentHardLimits,
) -> AgentEvent {
    debug_assert!(record.status.persisting_revision.is_none());
    let now = now_millis();
    record.status.updated_at = now;
    if let Some(turn) = record.runtime.turn.as_mut() {
        if turn.id == identity.turn_id && turn.generation == identity.generation {
            turn.updated_at = now;
        }
    }
    let sequence = AgentEventSequence::new(record.journal.next_event_sequence);
    let activity = record.runtime.activities.observe(sequence, &kind, now);
    record.journal.next_event_sequence = record
        .journal
        .next_event_sequence
        .checked_add(1)
        .expect("agent event sequence exhausted");
    record.status.revision = record
        .status
        .revision
        .checked_add(1)
        .expect("agent snapshot revision exhausted");
    let event = AgentEvent {
        session_id: identity.session_id,
        turn_id: Some(identity.turn_id),
        generation: identity.generation,
        sequence,
        activity,
        kind,
    };
    record.journal.events.push_back(event.clone());
    while record.journal.events.len() > limits.event_buffer_capacity {
        record.journal.events.pop_front();
    }
    event
}

pub(in crate::service) fn close_reasoning_for_terminal(
    record: &mut SessionRecord,
    identity: TurnIdentity,
    limits: AgentHardLimits,
) -> Result<Option<AgentEvent>, AgentError> {
    let reasoning = {
        let turn = exact_turn_mut(record, identity)?;
        let mut next = turn.reasoning;
        if !next
            .terminal_close(now_millis())
            .map_err(map_reasoning_error)?
        {
            return Ok(None);
        }
        turn.reasoning = next;
        turn.reasoning.snapshot()
    };
    Ok(Some(record_event(
        record,
        identity,
        AgentEventKind::ReasoningUpdated { reasoning },
        limits,
    )))
}

pub(in crate::service) fn map_reasoning_error(error: AgentReasoningTransitionError) -> AgentError {
    AgentError::Protocol(error.to_string())
}

pub(in crate::service) fn record_compaction_event(
    record: &mut SessionRecord,
    compaction: AgentCompactionSnapshot,
    limits: AgentHardLimits,
) -> AgentEvent {
    debug_assert!(record.status.persisting_revision.is_none());
    let now = now_millis();
    record.status.updated_at = now;
    let sequence = AgentEventSequence::new(record.journal.next_event_sequence);
    record.journal.next_event_sequence = record
        .journal
        .next_event_sequence
        .checked_add(1)
        .expect("agent event sequence exhausted");
    record.status.revision = record
        .status
        .revision
        .checked_add(1)
        .expect("agent snapshot revision exhausted");
    let event = AgentEvent {
        session_id: record.session_id,
        turn_id: None,
        generation: record.status.generation,
        sequence,
        activity: None,
        kind: AgentEventKind::CompactionUpdated { compaction },
    };
    record.journal.events.push_back(event.clone());
    while record.journal.events.len() > limits.event_buffer_capacity {
        record.journal.events.pop_front();
    }
    event
}

pub(in crate::service) fn record_session_event(
    record: &mut SessionRecord,
    kind: AgentEventKind,
    limits: AgentHardLimits,
) -> AgentEvent {
    debug_assert!(record.status.persisting_revision.is_none());
    let now = now_millis();
    record.status.updated_at = now;
    let sequence = AgentEventSequence::new(record.journal.next_event_sequence);
    let activity = record.runtime.activities.observe(sequence, &kind, now);
    record.journal.next_event_sequence = record
        .journal
        .next_event_sequence
        .checked_add(1)
        .expect("agent event sequence exhausted");
    record.status.revision = record
        .status
        .revision
        .checked_add(1)
        .expect("agent snapshot revision exhausted");
    let event = AgentEvent {
        session_id: record.session_id,
        turn_id: None,
        generation: record.status.generation,
        sequence,
        activity,
        kind,
    };
    record.journal.events.push_back(event.clone());
    while record.journal.events.len() > limits.event_buffer_capacity {
        record.journal.events.pop_front();
    }
    event
}
