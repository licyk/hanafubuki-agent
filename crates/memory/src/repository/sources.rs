use super::*;

impl AgentMemoryRepository {
    pub fn upsert_source_revision(
        &self,
        session_id: &str,
        source_revision: u64,
    ) -> Result<(), AgentMemoryError> {
        validate_id(session_id, "session id")?;
        let _guard = self.inner.gate.lock();
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO memory_sources(session_id, contribution_mode, mode_revision, source_revision)
                 VALUES(?1, 'disabled', 0, ?2)
                 ON CONFLICT(session_id) DO UPDATE SET source_revision=excluded.source_revision
                 WHERE excluded.source_revision > memory_sources.source_revision",
                params![session_id, to_i64(source_revision)?],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn sync_source_state(
        &self,
        session_id: &str,
        mode: AgentMemoryContributionMode,
        mode_revision: u64,
        source_revision: u64,
    ) -> Result<(), AgentMemoryError> {
        validate_id(session_id, "session id")?;
        let contribution_mode = mode.as_str();
        let revoke = !mode.contributes();
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let existing: Option<(String, i64, i64)> = connection
            .query_row(
                "SELECT contribution_mode, mode_revision, source_revision FROM memory_sources WHERE session_id=?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        if existing.as_ref().is_some_and(
            |(existing_mode, existing_mode_revision, existing_source_revision)| {
                existing_mode == contribution_mode
                    && from_i64(*existing_mode_revision).ok() == Some(mode_revision)
                    && from_i64(*existing_source_revision)
                        .ok()
                        .is_some_and(|revision| revision >= source_revision)
            },
        ) {
            return Ok(());
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO memory_sources(session_id, contribution_mode, mode_revision, source_revision)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(session_id) DO UPDATE SET
                   contribution_mode=excluded.contribution_mode,
                   mode_revision=excluded.mode_revision,
                   source_revision=max(memory_sources.source_revision, excluded.source_revision),
                   revoked=?5,
                   lease_token=NULL,
                   lease_expires=NULL
                 WHERE excluded.mode_revision >= memory_sources.mode_revision",
                params![
                    session_id,
                    contribution_mode,
                    to_i64(mode_revision)?,
                    to_i64(source_revision)?,
                    revoke
                ],
            )
            .map_err(db_error)?;
        if revoke {
            transaction
                .execute(
                    "DELETE FROM memory_stage1 WHERE session_id=?1",
                    [session_id],
                )
                .map_err(db_error)?;
        }
        transaction
            .execute("UPDATE memory_meta SET dirty=1 WHERE singleton=1", [])
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    pub fn contribution_mode(
        &self,
        session_id: &str,
    ) -> Result<(AgentMemoryContributionMode, u64), AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let connection = self.connection()?;
        let row: Option<(String, i64)> = connection
            .query_row(
                "SELECT contribution_mode, mode_revision FROM memory_sources WHERE session_id=?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        let (mode, revision) = row.ok_or(AgentMemoryError::NotFound)?;
        Ok((parse_mode(&mode), from_i64(revision)?))
    }

    pub fn revoke_source(&self, session_id: &str) -> Result<(), AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        transaction
            .execute(
                "UPDATE memory_sources SET revoked=1, lease_token=NULL, lease_expires=NULL WHERE session_id=?1",
                [session_id],
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "DELETE FROM memory_stage1 WHERE session_id=?1",
                [session_id],
            )
            .map_err(db_error)?;
        transaction
            .execute("UPDATE memory_meta SET dirty=1 WHERE singleton=1", [])
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(())
    }
}
