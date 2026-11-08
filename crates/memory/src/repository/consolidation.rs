use super::*;

use std::collections::BTreeSet;

impl AgentMemoryRepository {
    pub fn claim_consolidation(
        &self,
        now_millis: u64,
        lease_millis: u64,
    ) -> Result<Option<MemoryLease>, AgentMemoryError> {
        let token = self.new_token("global", now_millis);
        let expires = now_millis.saturating_add(lease_millis);
        let _guard = self.inner.gate.lock();
        self.install_pending_artifacts()?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let (reset_epoch, generation): (i64, i64) = transaction
            .query_row(
                "SELECT reset_epoch, generation FROM memory_meta WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db_error)?;
        let changed = transaction
            .execute(
                "UPDATE memory_meta SET global_lease_token=?1, global_lease_expires=?2
                 WHERE singleton=1 AND dirty=1
                   AND (global_next_retry_at IS NULL OR global_next_retry_at<=?3)
                   AND (global_lease_token IS NULL OR global_lease_expires<=?3)",
                params![token, to_i64(expires)?, to_i64(now_millis)?],
            )
            .map_err(db_error)?;
        if changed == 0 {
            transaction.commit().map_err(db_error)?;
            log::debug!("agent memory consolidation not claimed: nothing dirty, backing off or lease in use");
            return Ok(None);
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO memory_jobs(job_id, kind, scope, lease_token, reset_epoch, expires_at, attempt)
                 VALUES('phase2:global', 'phase2', 'global', ?1, ?2, ?3,
                   (SELECT global_retry_count + 1 FROM memory_meta WHERE singleton=1))",
                params![token, reset_epoch, to_i64(expires)?],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        log::debug!("agent memory consolidation claimed generation={generation} expires={expires}");
        Ok(Some(MemoryLease {
            scope: "global".into(),
            token,
            source_revision: from_i64(generation)?,
            reset_epoch: from_i64(reset_epoch)?,
            expires_at_millis: expires,
        }))
    }

    pub fn renew_consolidation(
        &self,
        lease: &MemoryLease,
        now_millis: u64,
        lease_millis: u64,
    ) -> Result<MemoryLease, AgentMemoryError> {
        let expires = now_millis.saturating_add(lease_millis);
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let job_changed = transaction
            .execute(
                "UPDATE memory_jobs SET expires_at=?1
                 WHERE job_id='phase2:global' AND kind='phase2' AND scope='global'
                   AND lease_token=?2 AND reset_epoch=?3 AND expires_at>?4",
                params![
                    to_i64(expires)?,
                    lease.token,
                    to_i64(lease.reset_epoch)?,
                    to_i64(now_millis)?
                ],
            )
            .map_err(db_error)?;
        if job_changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        let meta_changed = transaction
            .execute(
                "UPDATE memory_meta SET global_lease_expires=?1
                 WHERE singleton=1 AND global_lease_token=?2 AND reset_epoch=?3
                   AND generation=?4 AND global_lease_expires>?5",
                params![
                    to_i64(expires)?,
                    lease.token,
                    to_i64(lease.reset_epoch)?,
                    to_i64(lease.source_revision)?,
                    to_i64(now_millis)?
                ],
            )
            .map_err(db_error)?;
        if meta_changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        transaction.commit().map_err(db_error)?;
        let mut renewed = lease.clone();
        renewed.expires_at_millis = expires;
        Ok(renewed)
    }

    pub fn consolidation_inputs(
        &self,
        maximum: usize,
    ) -> Result<Vec<(String, u64, StageOneResult)>, AgentMemoryError> {
        let maximum = maximum.clamp(1, 4096);
        let _guard = self.inner.gate.lock();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT s.session_id, m.source_revision, m.raw_memory, m.rollout_summary, m.rollout_slug
                 FROM memory_stage1 m JOIN memory_sources s ON s.session_id=m.session_id
                 WHERE s.contribution_mode='enabled' AND s.revoked=0
                 ORDER BY m.usage_count DESC, COALESCE(m.last_used_at, 0) DESC,
                          m.generated_at DESC, s.session_id ASC LIMIT ?1",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([to_i64(maximum as u64)?], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    StageOneResult {
                        raw_memory: row.get(2)?,
                        rollout_summary: row.get(3)?,
                        rollout_slug: row.get(4)?,
                    },
                ))
            })
            .map_err(db_error)?
            .map(|row| {
                let (session, revision, result) = row.map_err(db_error)?;
                Ok((session, from_i64(revision)?, result))
            })
            .collect();
        rows
    }

    pub fn expire_unused_stage_one(
        &self,
        now_millis: u64,
        maximum_unused_days: u32,
    ) -> Result<usize, AgentMemoryError> {
        let cutoff = now_millis
            .saturating_sub(u64::from(maximum_unused_days).saturating_mul(24 * 60 * 60 * 1000));
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let removed = transaction
            .execute(
                "DELETE FROM memory_stage1 WHERE COALESCE(last_used_at, generated_at) < ?1",
                [to_i64(cutoff)?],
            )
            .map_err(db_error)?;
        if removed > 0 {
            transaction
                .execute("UPDATE memory_meta SET dirty=1 WHERE singleton=1", [])
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(removed)
    }

    pub fn fail_consolidation(
        &self,
        lease: &MemoryLease,
        now_millis: u64,
        bounded_error: &str,
    ) -> Result<(), AgentMemoryError> {
        let error = bound_string(bounded_error, 1024);
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        ensure_current_lease(&transaction, lease, now_millis, "phase2")?;
        let retry_count: i64 = transaction
            .query_row(
                "SELECT global_retry_count FROM memory_meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let next_retry_at = now_millis.saturating_add(retry_backoff_millis(retry_count));
        let changed = transaction
            .execute(
                "UPDATE memory_meta SET global_lease_token=NULL, global_lease_expires=NULL,
                   global_retry_count=global_retry_count+1, global_next_retry_at=?1, last_error=?2
                 WHERE singleton=1 AND reset_epoch=?3 AND global_lease_token=?4",
                params![
                    to_i64(next_retry_at)?,
                    error,
                    to_i64(lease.reset_epoch)?,
                    lease.token
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        transaction
            .execute(
                "DELETE FROM memory_jobs WHERE job_id='phase2:global' AND lease_token=?1",
                [&lease.token],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }

    pub fn publish_consolidation_workspace(
        &self,
        lease: &MemoryLease,
        workspace: &MemoryConsolidationWorkspace,
        now_millis: u64,
    ) -> Result<u64, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        ensure_current_lease(&transaction, lease, now_millis, "phase2")?;
        if workspace.generation != lease.source_revision.saturating_add(1) {
            return Err(AgentMemoryError::StaleRevision);
        }
        for source in &workspace.sources {
            let current: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM memory_sources s JOIN memory_stage1 m ON m.session_id=s.session_id
                     WHERE s.session_id=?1 AND s.contribution_mode='enabled' AND s.revoked=0
                       AND m.source_revision=?2 AND s.extracted_revision=?2",
                    params![source.session_id, to_i64(source.source_revision)?],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if current.is_none() {
                return Err(AgentMemoryError::StaleRevision);
            }
        }
        validate_selected_workspace_sources(
            &transaction,
            workspace,
            &self.inner.paths.ad_hoc_notes,
        )?;
        let current_generation: i64 = transaction
            .query_row(
                "SELECT generation FROM memory_meta WHERE singleton=1 AND reset_epoch=?1 AND global_lease_token=?2",
                params![to_i64(lease.reset_epoch)?, lease.token],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or(AgentMemoryError::LeaseLost)?;
        let next_generation = from_i64(current_generation)?.saturating_add(1);
        self.stage_artifacts(next_generation, workspace)?;
        let changed = transaction
            .execute(
                "UPDATE memory_meta SET generation=?1, dirty=0, global_lease_token=NULL,
                   global_lease_expires=NULL, global_retry_count=0, global_next_retry_at=NULL,
                   last_success_at=?2, last_error=NULL
                 WHERE singleton=1 AND reset_epoch=?3 AND global_lease_token=?4",
                params![
                    to_i64(next_generation)?,
                    to_i64(now_millis)?,
                    to_i64(lease.reset_epoch)?,
                    lease.token
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        transaction
            .execute(
                "DELETE FROM memory_jobs WHERE job_id='phase2:global' AND lease_token=?1",
                [&lease.token],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        // The commit is the publication: from here the staged generation is
        // the current one, installed now or by `install_pending_artifacts`.
        // So a failed install is not a failed consolidation, whose cleanup
        // would discard the only copy.
        if let Err(error) = self.install_artifacts(&workspace.path, next_generation) {
            log::warn!(
                "agent memory generation={next_generation} is committed but not yet installed: {error}"
            );
        }
        log::debug!("agent memory consolidation published generation={next_generation}");
        Ok(next_generation)
    }

    pub fn complete_consolidation_noop(
        &self,
        lease: &MemoryLease,
        workspace: &MemoryConsolidationWorkspace,
        now_millis: u64,
    ) -> Result<u64, AgentMemoryError> {
        if workspace.has_semantic_changes {
            return Err(AgentMemoryError::InvalidInput(
                "a changed memory workspace cannot use no-op completion".into(),
            ));
        }
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        ensure_current_lease(&transaction, lease, now_millis, "phase2")?;
        let current_generation: i64 = transaction
            .query_row(
                "SELECT generation FROM memory_meta WHERE singleton=1 AND reset_epoch=?1
                   AND global_lease_token=?2",
                params![to_i64(lease.reset_epoch)?, lease.token],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or(AgentMemoryError::LeaseLost)?;
        let current_generation = from_i64(current_generation)?;
        if workspace.generation != current_generation.saturating_add(1)
            || workspace.baseline_generation != Some(current_generation)
        {
            return Err(AgentMemoryError::StaleRevision);
        }
        self.validate_current_artifacts(current_generation)?;
        validate_selected_workspace_sources(
            &transaction,
            workspace,
            &self.inner.paths.ad_hoc_notes,
        )?;
        let changed = transaction
            .execute(
                "UPDATE memory_meta SET dirty=0, global_lease_token=NULL,
                   global_lease_expires=NULL, global_retry_count=0, global_next_retry_at=NULL,
                   last_success_at=?1, last_error=NULL
                 WHERE singleton=1 AND generation=?2 AND reset_epoch=?3
                   AND global_lease_token=?4",
                params![
                    to_i64(now_millis)?,
                    to_i64(current_generation)?,
                    to_i64(lease.reset_epoch)?,
                    lease.token
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        transaction
            .execute(
                "DELETE FROM memory_jobs WHERE job_id='phase2:global' AND lease_token=?1",
                [&lease.token],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        log::debug!("agent memory consolidation completed without semantic changes generation={current_generation}");
        Ok(current_generation)
    }
}

fn validate_selected_workspace_sources(
    transaction: &rusqlite::Transaction<'_>,
    workspace: &MemoryConsolidationWorkspace,
    note_path: &Path,
) -> Result<(), AgentMemoryError> {
    let limit = workspace.sources.len().max(1);
    let mut statement = transaction
        .prepare(
            "SELECT s.session_id, m.source_revision, m.raw_memory, m.rollout_summary, m.rollout_slug
             FROM memory_stage1 m JOIN memory_sources s ON s.session_id=m.session_id
             WHERE s.contribution_mode='enabled' AND s.revoked=0
             ORDER BY m.usage_count DESC, COALESCE(m.last_used_at, 0) DESC,
                      m.generated_at DESC, s.session_id ASC LIMIT ?1",
        )
        .map_err(db_error)?;
    let current = statement
        .query_map([to_i64(limit as u64)?], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(db_error)?
        .map(|row| {
            let (session_id, revision, raw_memory, rollout_summary, rollout_slug) =
                row.map_err(db_error)?;
            let source_revision = from_i64(revision)?;
            let rollout_path = format!("rollout_summaries/{session_id}-{rollout_slug}.md");
            let raw_memory_sha256 = hex_digest(raw_memory.as_bytes());
            let rollout_summary_sha256 = hex_digest(rollout_summary.as_bytes());
            let stage_one_id = super::artifacts::stage_one_id(
                &session_id,
                source_revision,
                &raw_memory_sha256,
                &rollout_summary_sha256,
                &rollout_path,
            )?;
            Ok((session_id, source_revision, stage_one_id))
        })
        .collect::<Result<Vec<_>, AgentMemoryError>>()?;
    let expected = workspace
        .sources
        .iter()
        .map(|source| {
            (
                source.session_id.clone(),
                source.source_revision,
                source.stage_one_id.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let actual = current.into_iter().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(AgentMemoryError::StaleRevision);
    }
    let mut current_notes =
        super::notes::active_ad_hoc_notes_from_connection(transaction, note_path)?;
    current_notes.sort_by(|left, right| left.note_id.cmp(&right.note_id));
    if current_notes != workspace.notes {
        return Err(AgentMemoryError::StaleRevision);
    }
    Ok(())
}
