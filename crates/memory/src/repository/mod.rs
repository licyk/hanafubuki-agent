use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use hanafubuki_agent_foundation::persistence::discard_sqlite;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::paths::io_error;
use crate::{
    AdHocMemoryNote, AgentMemoryContext, AgentMemoryContributionMode, AgentMemoryError,
    AgentMemoryPaths, AgentMemoryReadResult, AgentMemoryResource, AgentMemorySearchMatch,
    AgentMemoryStatus, MemoryConsolidationWorkspace, MemoryLease, MemoryWorkspaceSource,
    StageOneResult, MAX_MEMORY_RESOURCE_BYTES, MAX_MEMORY_SEARCH_RESULTS, MAX_MEMORY_SUMMARY_BYTES,
};

const SCHEMA_VERSION: i64 = 5;
/// How long a connection waits for a lock another one holds, like the
/// history index. Short in this crate's unit tests, which hold one on
/// purpose.
const BUSY_TIMEOUT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(200)
} else {
    std::time::Duration::from_secs(5)
};
const MAX_STAGE_ONE_TEXT_BYTES: usize = 256 * 1024;
const MAX_ARTIFACT_TOTAL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct AgentMemoryRepository {
    inner: Arc<RepositoryInner>,
}

struct RepositoryInner {
    paths: AgentMemoryPaths,
    gate: Mutex<()>,
    token_counter: AtomicU64,
}

mod artifacts;
mod consolidation;
mod notes;
mod phase_one;
mod reset;
mod resources;
mod schema;
mod sources;
mod usage;
mod validation;

use artifacts::hex_digest;
use validation::*;

impl AgentMemoryRepository {
    pub fn open(paths: AgentMemoryPaths) -> Result<Self, AgentMemoryError> {
        paths.prepare()?;
        let database_existed = paths.database.exists();
        let repository = Self {
            inner: Arc::new(RepositoryInner {
                paths,
                gate: Mutex::new(()),
                token_counter: AtomicU64::new(1),
            }),
        };
        // Only a database that is not one, is damaged or has another schema
        // is discarded (its contents are derived and rebuilt); a busy, locked
        // or unreadable one fails the open and is kept for the next.
        if let Err(error) = repository.initialize() {
            if !database_existed || !matches!(error, AgentMemoryError::Corrupt(_)) {
                return Err(error);
            }
            log::warn!(
                "discarding incompatible Agent memory derived state path={} error={error}",
                repository.inner.paths.database.display()
            );
            discard_sqlite(&repository.inner.paths.database).map_err(AgentMemoryError::Io)?;
            repository.discard_incompatible_derived_state()?;
            repository.initialize()?;
        }
        repository.install_pending_artifacts()?;
        repository.recover_staging()?;
        repository.recover_ad_hoc_notes()?;
        Ok(repository)
    }

    pub fn paths(&self) -> &AgentMemoryPaths {
        &self.inner.paths
    }

    pub(super) fn connection(&self) -> Result<Connection, AgentMemoryError> {
        let connection = Connection::open(&self.inner.paths.database).map_err(db_error)?;
        connection.busy_timeout(BUSY_TIMEOUT).map_err(db_error)?;
        Ok(connection)
    }

    fn discard_incompatible_derived_state(&self) -> Result<(), AgentMemoryError> {
        for directory in [
            &self.inner.paths.artifacts,
            &self.inner.paths.staging,
            &self.inner.paths.reset_trash,
        ] {
            if directory.exists() {
                fs::remove_dir_all(directory).map_err(io_error)?;
            }
            fs::create_dir_all(directory).map_err(io_error)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
