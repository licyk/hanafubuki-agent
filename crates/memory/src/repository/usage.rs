use super::validation::{db_error, to_i64, validate_id};
use super::AgentMemoryRepository;
use crate::{AgentMemoryError, MAX_USAGE_SOURCES};
use rusqlite::{params, TransactionBehavior};

impl AgentMemoryRepository {
    /// Counts a use of each cited source's current extraction.
    ///
    /// Recorded whatever the pipeline is doing: a citation names sources by
    /// session, and a source that was re-extracted or removed since is
    /// counted on its current row or not at all. A usage change that moves
    /// the ranked cutoff of a running consolidation fails its publish as
    /// stale, which the consolidation backoff retries.
    pub fn record_citation_usage(
        &self,
        source_session_ids: &[String],
        now_millis: u64,
    ) -> Result<(), AgentMemoryError> {
        if source_session_ids.len() > MAX_USAGE_SOURCES {
            return Err(AgentMemoryError::InvalidInput(
                "too many cited memory sources".into(),
            ));
        }
        let _guard = self.inner.gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        for session_id in source_session_ids {
            validate_id(session_id, "source session id")?;
            transaction
                .execute(
                    "UPDATE memory_stage1 SET usage_count=usage_count+1, last_used_at=?1
                     WHERE session_id=?2",
                    params![to_i64(now_millis)?, session_id],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)
    }
}
