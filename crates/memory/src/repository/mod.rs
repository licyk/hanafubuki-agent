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

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct AgentMemoryRepository {
    inner: Arc<RepositoryInner>,
}

struct RepositoryInner {
    paths: AgentMemoryPaths,
    gate: Mutex<()>,
    lifetime: Mutex<Option<hanafubuki_agent_foundation::file_lock::FileLock>>,
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
        use hanafubuki_agent_foundation::file_lock::FileLock;
        let _startup = FileLock::exclusive(&paths.root.join(".startup.lock")).map_err(io_error)?;
        let lifetime_path = paths.root.join(".store.lock");
        let exclusive = FileLock::try_exclusive(&lifetime_path).map_err(io_error)?;
        let alone = exclusive.is_some();
        paths.prepare()?;
        let database_existed = paths.database.exists();
        let repository = Self {
            inner: Arc::new(RepositoryInner {
                paths,
                gate: Mutex::new(()),
                lifetime: Mutex::new(if alone {
                    None
                } else {
                    Some(FileLock::shared(&lifetime_path).map_err(io_error)?)
                }),
            }),
        };
        // Only a database that is not one, is damaged or has another schema
        // is discarded (its contents are derived and rebuilt); a busy, locked
        // or unreadable one fails the open and is kept for the next.
        if let Err(error) = repository.initialize() {
            if !alone || !database_existed || !matches!(error, AgentMemoryError::Corrupt(_)) {
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
        if alone {
            repository.install_pending_artifacts()?;
            repository.recover_staging()?;
            drop(exclusive);
            *repository.inner.lifetime.lock() =
                Some(FileLock::shared(&lifetime_path).map_err(io_error)?);
        }
        repository.recover_ad_hoc_notes()?;
        Ok(repository)
    }

    pub fn paths(&self) -> &AgentMemoryPaths {
        &self.inner.paths
    }

    fn lock(&self) -> Result<RepositoryGuard<'_>, AgentMemoryError> {
        let local = self.inner.gate.lock();
        let file = hanafubuki_agent_foundation::file_lock::FileLock::exclusive(
            &self.inner.paths.root.join(".write.lock"),
        )
        .map_err(io_error)?;
        Ok(RepositoryGuard {
            _file: file,
            _local: local,
        })
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

struct RepositoryGuard<'a> {
    _file: hanafubuki_agent_foundation::file_lock::FileLock,
    _local: parking_lot::MutexGuard<'a, ()>,
}

#[cfg(test)]
mod tests;
