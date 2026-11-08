use super::*;

impl AgentMemoryRepository {
    pub fn reset(&self, expected_generation: u64) -> Result<u64, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let (generation, reset_epoch): (i64, i64) = transaction
            .query_row(
                "SELECT generation, reset_epoch FROM memory_meta WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db_error)?;
        if from_i64(generation)? != expected_generation {
            return Err(AgentMemoryError::StaleRevision);
        }
        let next_epoch = from_i64(reset_epoch)?.saturating_add(1);
        transaction
            .execute("DELETE FROM memory_jobs", [])
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM memory_stage1", [])
            .map_err(db_error)?;
        transaction
            .execute(
                "UPDATE memory_sources SET extracted_revision=0, lease_token=NULL,
                   lease_expires=NULL, retry_count=0, next_retry_at=NULL, last_error=NULL",
                [],
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "UPDATE memory_meta SET generation=0, reset_epoch=?1, dirty=0,
                   global_lease_token=NULL, global_lease_expires=NULL,
                   global_retry_count=0, global_next_retry_at=NULL,
                   last_success_at=NULL, last_error=NULL WHERE singleton=1",
                [to_i64(next_epoch)?],
            )
            .map_err(db_error)?;
        let trash = self
            .inner
            .paths
            .reset_trash
            .join(format!("reset-{next_epoch}"));
        if trash.exists() {
            fs::remove_dir_all(&trash).map_err(io_error)?;
        }
        if self.inner.paths.artifacts.exists() {
            fs::rename(&self.inner.paths.artifacts, &trash).map_err(io_error)?;
        }
        fs::create_dir_all(&self.inner.paths.artifacts).map_err(io_error)?;
        self.recover_staging_locked()?;
        transaction.commit().map_err(db_error)?;
        if trash.exists() {
            fs::remove_dir_all(trash).map_err(io_error)?;
        }
        log::debug!("agent memory reset completed reset_epoch={next_epoch}");
        Ok(0)
    }

    pub(super) fn recover_staging_locked(&self) -> Result<(), AgentMemoryError> {
        for entry in fs::read_dir(&self.inner.paths.staging).map_err(io_error)? {
            let path = entry.map_err(io_error)?.path();
            if path.is_dir() {
                fs::remove_dir_all(path).map_err(io_error)?;
            } else {
                fs::remove_file(path).map_err(io_error)?;
            }
        }
        Ok(())
    }

    pub fn generation(&self) -> Result<u64, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        self.published_generation()
    }

    /// The published generation, for a caller that already holds the gate.
    /// Provenance only; never an admission gate for a read.
    pub(super) fn published_generation(&self) -> Result<u64, AgentMemoryError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT generation FROM memory_meta WHERE singleton=1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_error)
            .and_then(from_i64)
    }

    pub fn status(&self, enabled: bool) -> Result<AgentMemoryStatus, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let connection = self.connection()?;
        let (generation, last_success, last_error): (i64, Option<i64>, Option<String>) = connection
            .query_row(
                "SELECT generation, last_success_at, last_error FROM memory_meta WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(db_error)?;
        let source_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM memory_sources WHERE contribution_mode='enabled' AND revoked=0",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let phase_one_count: i64 = connection
            .query_row("SELECT count(*) FROM memory_stage1", [], |row| row.get(0))
            .map_err(db_error)?;
        let running_jobs: i64 = connection
            .query_row("SELECT count(*) FROM memory_jobs", [], |row| row.get(0))
            .map_err(db_error)?;
        Ok(AgentMemoryStatus {
            enabled,
            generation: from_i64(generation)?,
            source_count: from_i64(source_count)?,
            phase_one_count: from_i64(phase_one_count)?,
            running_jobs: from_i64(running_jobs)?,
            last_success_at_millis: last_success.map(from_i64).transpose()?,
            last_error,
        })
    }

    pub(super) fn new_token(&self, scope: &str, now_millis: u64) -> String {
        let counter = self.inner.token_counter.fetch_add(1, Ordering::Relaxed);
        hex_digest(format!("{scope}:{now_millis}:{counter}").as_bytes())
    }
}
