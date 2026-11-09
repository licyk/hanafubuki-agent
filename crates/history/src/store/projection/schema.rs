use crate::store::*;

/// The index's `user_version`. An index of another version is discarded at
/// open and rebuilt from the rollouts (5 added `parent_session_id`).
const INDEX_SCHEMA_VERSION: i64 = 5;

impl FileAgentHistoryStore {
    pub(in crate::store) fn initialize_database(&self) -> Result<(), AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let schema_version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(database_error)?;
        if schema_version != 0 && schema_version != INDEX_SCHEMA_VERSION {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "unsupported Agent history index schema version {schema_version}"
            )));
        }
        connection
            .execute_batch(
                r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = FULL;
PRAGMA user_version = 5;

CREATE TABLE IF NOT EXISTS agent_store_state (
    key TEXT PRIMARY KEY,
    value INTEGER NOT NULL
);
INSERT OR IGNORE INTO agent_store_state(key, value) VALUES ('next_session_id', 1);

CREATE TABLE IF NOT EXISTS agent_sessions (
    session_id INTEGER PRIMARY KEY,
    rollout_path TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    lifecycle TEXT NOT NULL,
    generation INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    checkpoint_revision INTEGER NOT NULL,
    model TEXT NOT NULL,
    mode TEXT NOT NULL,
    title TEXT,
    latest_turn_id INTEGER,
    latest_turn_phase TEXT,
    recovery_needed INTEGER NOT NULL,
    runtime_json TEXT NOT NULL,
    summary_json TEXT NOT NULL,
    forked_from_session_id INTEGER,
    archived_at TEXT,
    parent_session_id INTEGER
);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_updated
    ON agent_sessions(lifecycle, updated_at DESC, session_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_created
    ON agent_sessions(lifecycle, created_at DESC, session_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_forked_from
    ON agent_sessions(forked_from_session_id);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_parent
    ON agent_sessions(parent_session_id, lifecycle, updated_at DESC, session_id DESC);

CREATE TABLE IF NOT EXISTS agent_turns (
    session_id INTEGER NOT NULL,
    turn_id INTEGER NOT NULL,
    first_record_ordinal INTEGER NOT NULL,
    first_item_order INTEGER NOT NULL,
    last_record_ordinal INTEGER,
    status TEXT NOT NULL,
    error_json TEXT,
    started_at TEXT,
    completed_at TEXT,
    final_answer_item_id INTEGER,
    PRIMARY KEY(session_id, turn_id)
);
CREATE INDEX IF NOT EXISTS idx_agent_turns_page
    ON agent_turns(session_id, first_item_order);

CREATE TABLE IF NOT EXISTS agent_history_items (
    session_id INTEGER NOT NULL,
    turn_id INTEGER,
    item_id INTEGER NOT NULL,
    first_record_ordinal INTEGER NOT NULL,
    first_item_order INTEGER NOT NULL,
    item_revision INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    item_type TEXT NOT NULL,
    item_json TEXT NOT NULL,
    session_revision INTEGER NOT NULL,
    PRIMARY KEY(session_id, item_id)
);
CREATE INDEX IF NOT EXISTS idx_agent_history_items_page
    ON agent_history_items(session_id, first_item_order);
CREATE INDEX IF NOT EXISTS idx_agent_history_items_turn_page
    ON agent_history_items(session_id, turn_id, first_item_order);
CREATE INDEX IF NOT EXISTS idx_agent_history_user_messages
    ON agent_history_items(session_id, first_item_order)
    WHERE item_type = 'user_message';

CREATE TABLE IF NOT EXISTS agent_history_records (
    session_id INTEGER NOT NULL,
    record_ordinal INTEGER NOT NULL,
    start_byte_offset INTEGER NOT NULL,
    end_byte_offset INTEGER NOT NULL,
    session_revision INTEGER NOT NULL,
    PRIMARY KEY(session_id, record_ordinal)
);

CREATE TABLE IF NOT EXISTS agent_history_projection_state (
    session_id INTEGER PRIMARY KEY,
    next_jsonl_byte_offset INTEGER NOT NULL,
    next_ordinal INTEGER NOT NULL,
    last_projected_session_revision INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_projection_repairs (
    session_id INTEGER PRIMARY KEY,
    rollout_path TEXT NOT NULL,
    requested_revision INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_session_lineage (
    child_session_id INTEGER PRIMARY KEY,
    parent_session_id INTEGER NOT NULL,
    parent_end_ordinal_exclusive INTEGER NOT NULL,
    parent_end_byte_offset INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_lineage_parent
    ON agent_session_lineage(parent_session_id);
                "#,
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub(in crate::store) fn connection(&self) -> Result<Connection, AgentHistoryStoreError> {
        let connection =
            Connection::open(&self.inner.paths.sqlite_database).map_err(database_error)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(database_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(database_error)?;
        Ok(connection)
    }

    pub(in crate::store) fn allocate_session_id_inner(
        &self,
    ) -> Result<AgentSessionId, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let next: i64 = transaction
            .query_row(
                "SELECT value FROM agent_store_state WHERE key = 'next_session_id'",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let allocated = from_i64(next, "next session id")?;
        let following = allocated.checked_add(1).ok_or_else(|| {
            AgentHistoryStoreError::CorruptHistory("session id exhausted".to_string())
        })?;
        transaction
            .execute(
                "UPDATE agent_store_state SET value = ? WHERE key = 'next_session_id'",
                params![to_i64(following, "next session id")?],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        Ok(AgentSessionId::new(allocated))
    }
}
