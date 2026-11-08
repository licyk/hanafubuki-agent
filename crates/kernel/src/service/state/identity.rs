use super::ensure_no_staged_commit;
use crate::model::{AgentError, AgentGeneration, AgentTurnId};
use crate::service::{
    CompactionIdentity, CompactionRecord, SessionRecord, TurnIdentity, TurnRecord,
};

pub(in crate::service) fn exact_turn(
    record: &SessionRecord,
    turn_id: AgentTurnId,
    generation: AgentGeneration,
) -> Result<&TurnRecord, AgentError> {
    let turn = record
        .runtime
        .turn
        .as_ref()
        .ok_or(AgentError::TurnNotRunning)?;
    if turn.id != turn_id || turn.generation != generation {
        return Err(AgentError::StaleGeneration);
    }
    Ok(turn)
}

pub(in crate::service) fn exact_compaction(
    record: &SessionRecord,
    identity: CompactionIdentity,
) -> Result<&CompactionRecord, AgentError> {
    let compaction = record
        .runtime
        .compaction
        .as_ref()
        .ok_or(AgentError::CompactionNotRunning)?;
    if compaction.snapshot.id != identity.id
        || compaction.snapshot.generation != identity.generation
    {
        return Err(AgentError::StaleCompaction);
    }
    Ok(compaction)
}

pub(in crate::service) fn exact_compaction_mut(
    record: &mut SessionRecord,
    identity: CompactionIdentity,
) -> Result<&mut CompactionRecord, AgentError> {
    ensure_no_staged_commit(record)?;
    let compaction = record
        .runtime
        .compaction
        .as_mut()
        .ok_or(AgentError::CompactionNotRunning)?;
    if compaction.snapshot.id != identity.id
        || compaction.snapshot.generation != identity.generation
    {
        return Err(AgentError::StaleCompaction);
    }
    Ok(compaction)
}

pub(in crate::service) fn exact_turn_identity(
    record: &SessionRecord,
    identity: TurnIdentity,
) -> Result<&TurnRecord, AgentError> {
    exact_turn(record, identity.turn_id, identity.generation)
}

pub(in crate::service) fn exact_turn_mut(
    record: &mut SessionRecord,
    identity: TurnIdentity,
) -> Result<&mut TurnRecord, AgentError> {
    ensure_no_staged_commit(record)?;
    let turn = record
        .runtime
        .turn
        .as_mut()
        .ok_or(AgentError::TurnNotRunning)?;
    if turn.id != identity.turn_id || turn.generation != identity.generation {
        return Err(AgentError::StaleGeneration);
    }
    Ok(turn)
}
