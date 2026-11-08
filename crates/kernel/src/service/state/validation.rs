use crate::model::{AgentError, AgentGeneration, AgentRevision};
use crate::service::SessionRecord;

pub(in crate::service) fn validate_session_mutation(
    record: &SessionRecord,
    generation: AgentGeneration,
    revision: AgentRevision,
) -> Result<(), AgentError> {
    ensure_no_staged_commit(record)?;
    if record.status.generation != generation {
        return Err(AgentError::StaleGeneration);
    }
    if record.status.revision != revision.value() {
        return Err(AgentError::StaleRevision);
    }
    if record.status.disposing
        || record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
    {
        return Err(AgentError::SessionMutationNotConfirmed);
    }
    if record
        .runtime
        .compaction
        .as_ref()
        .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
    {
        return Err(AgentError::CompactionAlreadyRunning);
    }
    Ok(())
}

pub(in crate::service) fn ensure_no_staged_commit(
    record: &SessionRecord,
) -> Result<(), AgentError> {
    if record.status.persisting_revision.is_some() {
        Err(AgentError::SessionMutationNotConfirmed)
    } else {
        Ok(())
    }
}

/// Whether `record` runs a turn or a compaction.
pub(in crate::service) fn has_active_operation(record: &SessionRecord) -> bool {
    record
        .runtime
        .turn
        .as_ref()
        .is_some_and(|turn| !turn.phase.is_terminal())
        || record
            .runtime
            .compaction
            .as_ref()
            .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
}

pub(in crate::service) fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(in crate::service) fn now_millis_u64() -> u64 {
    u64::try_from(now_millis()).unwrap_or(u64::MAX)
}
