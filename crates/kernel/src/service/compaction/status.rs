//! A compaction's status: `Reserved` when it begins, `Running` until the
//! model answers, `ReadyToInstall` until the install makes it `Completed`,
//! or `Failed` or `Cancelled` at any step.

use super::*;

impl Drop for CompactionExecutionGuard {
    fn drop(&mut self) {
        if self.armed {
            self.service.finish_compaction_terminal_on_drop(
                self.identity,
                AgentCompactionStatus::Cancelled,
            );
        }
    }
}

impl AgentService {
    pub(super) async fn set_compaction_status(
        &self,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
        summary: Option<String>,
        usage: Option<AgentCompactionUsage>,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let staged = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let compaction = exact_compaction_mut(record, identity)?;
            compaction.snapshot.status = status;
            compaction.snapshot.summary = summary;
            compaction.snapshot.usage = usage;
            compaction.snapshot.updated_at = now_millis_u64();
            let snapshot = record
                .runtime
                .compaction
                .as_ref()
                .expect("checked")
                .snapshot
                .clone();
            record_compaction_event(record, snapshot, self.inner.limits);
            record
                .runtime
                .compaction
                .as_mut()
                .expect("checked")
                .operation_revision = record.status.revision;
            StagedPersistence::commit(&cell, record, previous)
        };
        self.persist_staged(staged).await?;
        Ok(())
    }

    /// Commits the compaction's end and returns its event; nothing when the
    /// session or the compaction is gone or the compaction already ended.
    pub(super) async fn finish_compaction_terminal(
        &self,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
        error: Option<String>,
    ) -> Result<Option<AgentEvent>, AgentError> {
        let Some(cell) = self.inner.sessions.find(identity.session_id) else {
            return Ok(None);
        };
        let terminal = {
            let Ok(mut record) = cell.lock_after_persistence().await else {
                return Ok(None);
            };
            self.stage_compaction_terminal(&cell, &mut record, identity, status, error)?
        };
        let Some((staged, event)) = terminal else {
            return Ok(None);
        };
        self.persist_staged(staged).await?;
        cell.notify_terminal();
        Ok(Some(event))
    }

    /// `finish_compaction_terminal` for the drop guard, which can neither
    /// await nor wait: the terminal state is staged now, or once the commit
    /// in flight settles, and handed to the history store; its
    /// acknowledgement publishes it and wakes the terminal waiters.
    pub(super) fn finish_compaction_terminal_on_drop(
        &self,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
    ) {
        let Some(cell) = self.inner.sessions.find(identity.session_id) else {
            return;
        };
        let service = self.clone();
        let follow_up = cell.stage_or_defer(Box::new(move |cell, record| {
            let (staged, _) = service
                .stage_compaction_terminal(cell, record, identity, status, None)
                .unwrap_or_else(|error| {
                    log::warn!(
                        "agent_diag stage=compaction.drop_terminal session_id={} outcome=error error={}",
                        identity.session_id,
                        bounded_diagnostic(error.to_string())
                    );
                    None
                })?;
            let cell = cell.clone();
            Some(staged.into_follow_up(move || cell.notify_terminal()))
        }));
        self.hand_off(follow_up);
    }

    fn stage_compaction_terminal(
        &self,
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
        error: Option<String>,
    ) -> Result<Option<(StagedPersistence, AgentEvent)>, AgentError> {
        let Ok(compaction) = exact_compaction_mut(record, identity) else {
            return Ok(None);
        };
        if compaction.snapshot.status.is_terminal() {
            return Ok(None);
        }
        let previous = record.rollback_state();
        let compaction = exact_compaction_mut(record, identity)?;
        compaction.snapshot.status = status;
        compaction.snapshot.last_error = error.map(bounded_diagnostic);
        compaction.snapshot.updated_at = now_millis_u64();
        let snapshot = record
            .runtime
            .compaction
            .as_ref()
            .expect("checked")
            .snapshot
            .clone();
        let event = record_compaction_event(record, snapshot, self.inner.limits);
        record
            .runtime
            .compaction
            .as_mut()
            .expect("checked")
            .operation_revision = record.status.revision;
        Ok(Some((
            StagedPersistence::commit(cell, record, previous),
            event,
        )))
    }
}

/// Records `reserved` as the session's compaction and its event.
pub(super) fn reserve_compaction(
    record: &mut SessionRecord,
    reserved: AgentCompactionSnapshot,
    cancellation: CancellationToken,
    limits: AgentHardLimits,
) {
    record.status.updated_at = now_millis();
    record.runtime.compaction = Some(CompactionRecord {
        snapshot: reserved.clone(),
        cancellation,
        operation_revision: record.status.revision,
    });
    record_compaction_event(record, reserved, limits);
    if let Some(compaction) = record.runtime.compaction.as_mut() {
        compaction.operation_revision = record.status.revision;
    }
}

pub(super) fn compaction_usage(sample: &CompactionSample) -> AgentCompactionUsage {
    match sample.reported_usage {
        Some(usage) => AgentCompactionUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            source: AgentUsageSource::ProviderReported,
        },
        None => AgentCompactionUsage {
            input_tokens: sample.estimated_input_tokens,
            output_tokens: sample.estimated_output_tokens,
            source: AgentUsageSource::Estimated,
        },
    }
}
