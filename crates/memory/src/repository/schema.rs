use super::validation::schema_error;
use super::{AgentMemoryRepository, SCHEMA_VERSION};
use crate::paths::io_error;
use crate::AgentMemoryError;
use std::fs;

impl AgentMemoryRepository {
    pub(super) fn initialize(&self) -> Result<(), AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        let connection = self.connection()?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA foreign_keys=ON;
                 CREATE TABLE IF NOT EXISTS memory_meta (
                   singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                   schema_version INTEGER NOT NULL,
                   generation INTEGER NOT NULL,
                   reset_epoch INTEGER NOT NULL,
                   dirty INTEGER NOT NULL,
                   global_lease_token TEXT,
                   global_lease_expires INTEGER,
                   last_success_at INTEGER,
                   last_error TEXT,
                   global_retry_count INTEGER NOT NULL DEFAULT 0,
                   global_next_retry_at INTEGER
                 );
                 CREATE TABLE IF NOT EXISTS memory_sources (
                   session_id TEXT PRIMARY KEY,
                   contribution_mode TEXT NOT NULL,
                   mode_revision INTEGER NOT NULL,
                   source_revision INTEGER NOT NULL DEFAULT 0,
                   extracted_revision INTEGER NOT NULL DEFAULT 0,
                   revoked INTEGER NOT NULL DEFAULT 0,
                   lease_token TEXT,
                   lease_expires INTEGER,
                   retry_count INTEGER NOT NULL DEFAULT 0,
                   next_retry_at INTEGER,
                   last_error TEXT
                 );
                 CREATE TABLE IF NOT EXISTS memory_stage1 (
                   session_id TEXT PRIMARY KEY REFERENCES memory_sources(session_id) ON DELETE CASCADE,
                   source_revision INTEGER NOT NULL,
                   raw_memory TEXT NOT NULL,
                   rollout_summary TEXT NOT NULL,
                   rollout_slug TEXT NOT NULL,
                   generated_at INTEGER NOT NULL,
                   usage_count INTEGER NOT NULL DEFAULT 0,
                   last_used_at INTEGER
                 );
                 CREATE TABLE IF NOT EXISTS memory_jobs (
                   job_id TEXT PRIMARY KEY,
                   kind TEXT NOT NULL,
                   scope TEXT NOT NULL,
                   lease_token TEXT NOT NULL,
                   reset_epoch INTEGER NOT NULL,
                   expires_at INTEGER NOT NULL,
                   attempt INTEGER NOT NULL,
                   last_error TEXT
                 );",
            )
            .map_err(schema_error)?;
        connection
            .execute(
                "INSERT OR IGNORE INTO memory_meta(singleton, schema_version, generation, reset_epoch, dirty)
                 VALUES(1, ?1, 0, 0, 0)",
                [SCHEMA_VERSION],
            )
            .map_err(schema_error)?;
        let mut version: i64 = connection
            .query_row(
                "SELECT schema_version FROM memory_meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(schema_error)?;
        if version == 4 {
            migrate_from_version_four(&connection)?;
            version = 5;
        }
        if version != SCHEMA_VERSION {
            return Err(AgentMemoryError::Corrupt(format!(
                "unsupported schema version {version}"
            )));
        }
        Ok(())
    }

    pub(super) fn recover_staging(&self) -> Result<(), AgentMemoryError> {
        for entry in fs::read_dir(&self.inner.paths.staging).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let metadata = entry.file_type().map_err(io_error)?;
            if metadata.is_symlink() {
                return Err(AgentMemoryError::Corrupt(
                    "memory staging contains a symbolic link".into(),
                ));
            }
            if metadata.is_dir() {
                fs::remove_dir_all(entry.path()).map_err(io_error)?;
            } else {
                fs::remove_file(entry.path()).map_err(io_error)?;
            }
        }
        Ok(())
    }
}

/// Version 4 lacked the consolidation backoff. Its data is kept: the
/// artifacts are the user's memory and are discarded only with a database
/// this build cannot read.
fn migrate_from_version_four(connection: &rusqlite::Connection) -> Result<(), AgentMemoryError> {
    connection
        .execute_batch(
            "BEGIN IMMEDIATE;
             ALTER TABLE memory_meta ADD COLUMN global_retry_count INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE memory_meta ADD COLUMN global_next_retry_at INTEGER;
             UPDATE memory_meta SET schema_version=5 WHERE singleton=1 AND schema_version=4;
             COMMIT;",
        )
        .map_err(schema_error)
}
