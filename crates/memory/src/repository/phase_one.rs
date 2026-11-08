use super::*;

impl AgentMemoryRepository {
    pub fn claim_source(
        &self,
        session_id: &str,
        source_revision: u64,
        now_millis: u64,
        lease_millis: u64,
    ) -> Result<Option<MemoryLease>, AgentMemoryError> {
        validate_id(session_id, "session id")?;
        let token = self.new_token(session_id, now_millis);
        let expires = now_millis.saturating_add(lease_millis);
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO memory_sources(session_id, contribution_mode, mode_revision, source_revision)
                 VALUES(?1, 'disabled', 0, ?2)",
                params![session_id, to_i64(source_revision)?],
            )
            .map_err(db_error)?;
        let reset_epoch: i64 = transaction
            .query_row(
                "SELECT reset_epoch FROM memory_meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let changed = transaction
            .execute(
                "UPDATE memory_sources SET source_revision=?1, lease_token=?2, lease_expires=?3
                 WHERE session_id=?4 AND contribution_mode='enabled' AND revoked=0
                   AND extracted_revision < ?1
                   AND (next_retry_at IS NULL OR next_retry_at <= ?5)
                   AND (lease_token IS NULL OR lease_expires <= ?5)",
                params![
                    to_i64(source_revision)?,
                    token,
                    to_i64(expires)?,
                    session_id,
                    to_i64(now_millis)?
                ],
            )
            .map_err(db_error)?;
        if changed == 0 {
            transaction.commit().map_err(db_error)?;
            return Ok(None);
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO memory_jobs(job_id, kind, scope, lease_token, reset_epoch, expires_at, attempt)
                 VALUES(?1, 'phase1', ?2, ?3, ?4, ?5,
                   COALESCE((SELECT retry_count + 1 FROM memory_sources WHERE session_id=?2), 1))",
                params![
                    format!("phase1:{session_id}"),
                    session_id,
                    token,
                    reset_epoch,
                    to_i64(expires)?
                ],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(Some(MemoryLease {
            scope: session_id.to_string(),
            token,
            source_revision,
            reset_epoch: from_i64(reset_epoch)?,
            expires_at_millis: expires,
        }))
    }

    pub fn renew_source(
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
                 WHERE kind='phase1' AND scope=?2 AND lease_token=?3
                   AND reset_epoch=?4 AND expires_at>?5",
                params![
                    to_i64(expires)?,
                    lease.scope,
                    lease.token,
                    to_i64(lease.reset_epoch)?,
                    to_i64(now_millis)?
                ],
            )
            .map_err(db_error)?;
        if job_changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        let source_changed = transaction
            .execute(
                "UPDATE memory_sources SET lease_expires=?1
                 WHERE session_id=?2 AND lease_token=?3 AND source_revision=?4
                   AND lease_expires>?5",
                params![
                    to_i64(expires)?,
                    lease.scope,
                    lease.token,
                    to_i64(lease.source_revision)?,
                    to_i64(now_millis)?
                ],
            )
            .map_err(db_error)?;
        if source_changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        transaction.commit().map_err(db_error)?;
        let mut renewed = lease.clone();
        renewed.expires_at_millis = expires;
        Ok(renewed)
    }

    pub fn complete_source(
        &self,
        lease: &MemoryLease,
        result: Option<StageOneResult>,
        now_millis: u64,
    ) -> Result<(), AgentMemoryError> {
        if let Some(result) = &result {
            validate_text(&result.raw_memory, MAX_STAGE_ONE_TEXT_BYTES, "raw memory")?;
            validate_text(
                &result.rollout_summary,
                MAX_STAGE_ONE_TEXT_BYTES,
                "rollout summary",
            )?;
            validate_text(&result.rollout_slug, 80, "rollout slug")?;
        }
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        ensure_current_lease(&transaction, lease, now_millis, "phase1")?;
        if let Some(result) = result {
            transaction
                .execute(
                    "INSERT INTO memory_stage1(session_id, source_revision, raw_memory, rollout_summary, rollout_slug, generated_at)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(session_id) DO UPDATE SET source_revision=excluded.source_revision,
                       raw_memory=excluded.raw_memory, rollout_summary=excluded.rollout_summary,
                       rollout_slug=excluded.rollout_slug, generated_at=excluded.generated_at",
                    params![
                        lease.scope,
                        to_i64(lease.source_revision)?,
                        result.raw_memory,
                        result.rollout_summary,
                        result.rollout_slug,
                        to_i64(now_millis)?
                    ],
                )
                .map_err(db_error)?;
        } else {
            transaction
                .execute(
                    "DELETE FROM memory_stage1 WHERE session_id=?1",
                    [&lease.scope],
                )
                .map_err(db_error)?;
        }
        let changed = transaction
            .execute(
                "UPDATE memory_sources SET extracted_revision=?1, lease_token=NULL,
                   lease_expires=NULL, retry_count=0, next_retry_at=NULL, last_error=NULL
                 WHERE session_id=?2 AND lease_token=?3",
                params![to_i64(lease.source_revision)?, lease.scope, lease.token],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(AgentMemoryError::LeaseLost);
        }
        transaction
            .execute(
                "DELETE FROM memory_jobs WHERE scope=?1 AND lease_token=?2",
                params![lease.scope, lease.token],
            )
            .map_err(db_error)?;
        transaction
            .execute("UPDATE memory_meta SET dirty=1 WHERE singleton=1", [])
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    pub fn fail_source(
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
        ensure_current_lease(&transaction, lease, now_millis, "phase1")?;
        let retry_count: i64 = transaction
            .query_row(
                "SELECT retry_count FROM memory_sources WHERE session_id=?1",
                [&lease.scope],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let backoff = retry_backoff_millis(retry_count);
        transaction
            .execute(
                "UPDATE memory_sources SET lease_token=NULL, lease_expires=NULL,
                   retry_count=retry_count+1, next_retry_at=?1, last_error=?2
                 WHERE session_id=?3 AND lease_token=?4",
                params![
                    to_i64(now_millis.saturating_add(backoff))?,
                    error,
                    lease.scope,
                    lease.token
                ],
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "DELETE FROM memory_jobs WHERE scope=?1 AND lease_token=?2",
                params![lease.scope, lease.token],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(())
    }
}
