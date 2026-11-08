use std::sync::Arc;

use hanafubuki_agent_contracts::{AgentMemoryContributionCause, AgentNotification};

use crate::model::{
    AgentError, AgentMemoryContributionMode, AgentMemoryContributionState, AgentRevision,
    AgentSessionId,
};
use crate::service::state::{ensure_no_staged_commit, now_millis};
use crate::service::{AgentService, SessionCell, SessionRecord, StagedPersistence};

/// A staged change of the session's memory contribution.
struct StagedContribution {
    staged: StagedPersistence,
    state: AgentMemoryContributionState,
    revision: AgentRevision,
}

impl AgentService {
    pub fn session_memory_contribution(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentMemoryContributionState, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok(record.settings.memory_contribution)
    }

    /// The user's toggle of the session's memory contribution.
    pub fn set_session_memory_contribution(
        &self,
        session_id: AgentSessionId,
        expected_revision: u64,
        enabled: bool,
    ) -> Result<AgentMemoryContributionState, AgentError> {
        self.change_session_memory_contribution(
            session_id,
            expected_revision,
            enabled,
            AgentMemoryContributionCause::User,
        )
    }

    /// The session's memory contribution as the memory settings set it when
    /// the session is created or forked.
    pub fn initialize_session_memory_contribution(
        &self,
        session_id: AgentSessionId,
        expected_revision: u64,
        enabled: bool,
    ) -> Result<AgentMemoryContributionState, AgentError> {
        self.change_session_memory_contribution(
            session_id,
            expected_revision,
            enabled,
            AgentMemoryContributionCause::Initial,
        )
    }

    fn change_session_memory_contribution(
        &self,
        session_id: AgentSessionId,
        expected_revision: u64,
        enabled: bool,
        cause: AgentMemoryContributionCause,
    ) -> Result<AgentMemoryContributionState, AgentError> {
        let mode = if enabled {
            AgentMemoryContributionMode::Enabled
        } else {
            AgentMemoryContributionMode::Disabled
        };
        let cell = self.session(session_id)?;
        let change = {
            let mut record = cell.lock_after_persistence_blocking()?;
            stage_memory_contribution(&cell, &mut record, expected_revision, mode)?
        };
        match change {
            Ok(change) => {
                self.persist_staged_blocking(change.staged)?;
                self.notify_memory_contribution(session_id, change.state, cause, change.revision);
                Ok(change.state)
            }
            Err(unchanged) => Ok(unchanged),
        }
    }

    /// Marks a contributing session polluted after a tool output it cannot
    /// contribute was committed.
    pub(in crate::service) async fn mark_session_memory_contribution_polluted(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentMemoryContributionState, AgentError> {
        let cell = self.session(session_id)?;
        let change = {
            let mut record = cell.lock_after_persistence().await?;
            let current = record.settings.memory_contribution;
            if current.mode != AgentMemoryContributionMode::Enabled {
                return Ok(current);
            }
            stage_memory_contribution(
                &cell,
                &mut record,
                current.revision,
                AgentMemoryContributionMode::Polluted,
            )?
        };
        match change {
            Ok(change) => {
                self.persist_staged(change.staged).await?;
                self.notify_memory_contribution(
                    session_id,
                    change.state,
                    AgentMemoryContributionCause::Polluted,
                    change.revision,
                );
                Ok(change.state)
            }
            Err(unchanged) => Ok(unchanged),
        }
    }

    fn notify_memory_contribution(
        &self,
        session_id: AgentSessionId,
        state: AgentMemoryContributionState,
        cause: AgentMemoryContributionCause,
        revision: AgentRevision,
    ) {
        self.inner
            .registries
            .observers
            .notify(&AgentNotification::MemoryContributionChanged {
                session_id,
                state,
                cause,
                revision,
            });
    }
}

/// Stages `mode` as the session's memory contribution, or returns the
/// current state (`Err`) when it already is.
fn stage_memory_contribution(
    cell: &Arc<SessionCell>,
    record: &mut SessionRecord,
    expected_revision: u64,
    mode: AgentMemoryContributionMode,
) -> Result<Result<StagedContribution, AgentMemoryContributionState>, AgentError> {
    ensure_no_staged_commit(record)?;
    if record.settings.memory_contribution.revision != expected_revision {
        return Err(AgentError::StaleRevision);
    }
    if record.settings.memory_contribution.mode == AgentMemoryContributionMode::Polluted
        && mode != AgentMemoryContributionMode::Polluted
    {
        return Err(AgentError::InvalidInput(
            "a polluted Agent session cannot contribute memory again".to_string(),
        ));
    }
    if record.settings.memory_contribution.mode == mode {
        return Ok(Err(record.settings.memory_contribution));
    }
    let previous = record.rollback_state();
    record.settings.memory_contribution = AgentMemoryContributionState {
        mode,
        revision: record
            .settings
            .memory_contribution
            .revision
            .checked_add(1)
            .expect("Agent memory contribution revision exhausted"),
    };
    record.status.revision = record
        .status
        .revision
        .checked_add(1)
        .expect("Agent session revision exhausted");
    record.status.updated_at = now_millis();
    let state = record.settings.memory_contribution;
    let revision = AgentRevision::new(record.status.revision);
    Ok(Ok(StagedContribution {
        staged: StagedPersistence::commit(cell, record, previous),
        state,
        revision,
    }))
}
