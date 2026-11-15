use crate::store::*;

impl FileAgentHistoryStore {
    /// [`Self::open`] with the SQLite index discarded first, so it is
    /// rebuilt from the JSONL rollouts, which stay the durable history: the
    /// user's repair when the index is wrong in a way open does not detect.
    pub fn open_rebuilding_index(
        paths: AgentHistoryPaths,
        workers: BackgroundWorkerGroup,
        refusals: Arc<dyn AgentHistoryRefusalSink>,
    ) -> Result<Self, AgentHistoryStoreError> {
        Self::open_coordinated(paths, workers, refusals, true)
    }

    /// [`Self::open_reporting_refusals`] whose later refusals are only
    /// logged.
    pub fn open(
        paths: AgentHistoryPaths,
        workers: BackgroundWorkerGroup,
    ) -> Result<Self, AgentHistoryStoreError> {
        Self::open_reporting_refusals(paths, workers, Arc::new(crate::NoopAgentHistoryRefusalSink))
    }

    /// Opens the store; a session it refuses once open (the session list
    /// cannot decode its row) is told to `refusals`. What open itself
    /// refuses goes to recovery (`load_recovery_candidates`).
    pub fn open_reporting_refusals(
        paths: AgentHistoryPaths,
        workers: BackgroundWorkerGroup,
        refusals: Arc<dyn AgentHistoryRefusalSink>,
    ) -> Result<Self, AgentHistoryStoreError> {
        Self::open_coordinated(paths, workers, refusals, false)
    }

    fn open_coordinated(
        paths: AgentHistoryPaths,
        workers: BackgroundWorkerGroup,
        refusals: Arc<dyn AgentHistoryRefusalSink>,
        rebuild: bool,
    ) -> Result<Self, AgentHistoryStoreError> {
        let _startup = FileLock::exclusive(&paths.root.join(".startup.lock")).map_err(io_error)?;
        let lifetime_path = paths.root.join(".store.lock");
        let exclusive = FileLock::try_exclusive(&lifetime_path).map_err(io_error)?;
        let alone = exclusive.is_some();
        if rebuild {
            if !alone {
                return Err(AgentHistoryStoreError::Io(
                    "close the other Agents before rebuilding the shared history index".into(),
                ));
            }
            discard_sqlite(&paths.sqlite_database).map_err(AgentHistoryStoreError::Io)?;
        }
        let database_existed = paths.sqlite_database.exists();
        fs::create_dir_all(&paths.active_sessions).map_err(io_error)?;
        fs::create_dir_all(&paths.archived_sessions).map_err(io_error)?;
        fs::create_dir_all(&paths.trash).map_err(io_error)?;
        fs::create_dir_all(&paths.attachments).map_err(io_error)?;
        let store = Self {
            inner: Arc::new(StoreInner {
                paths,
                lifetime: Mutex::new(if alone {
                    None
                } else {
                    Some(FileLock::shared(&lifetime_path).map_err(io_error)?)
                }),
                session_owners: Mutex::new(HashMap::new()),
                workers,
                writers: Mutex::new(HashMap::new()),
                writer_gates: Mutex::new(HashMap::new()),
                database_gate: Mutex::new(()),
                projection_lag: Mutex::new(HashMap::new()),
                quarantined_references: Mutex::new(None),
                refusals,
                accepting: AtomicBool::new(true),
                #[cfg(test)]
                panic_next_append: AtomicBool::new(false),
            }),
        };
        if alone {
            store.cleanup_attachment_staging()?;
            store.recover_delete_obligations()?;
        }
        let initialize = || -> Result<(), AgentHistoryStoreError> {
            store.initialize_database()?;
            if database_existed {
                store.catch_up_pending_repairs()?;
            } else {
                store.catch_up_discovered_sessions()?;
            }
            if alone {
                store.reconcile_session_locations()?;
                store.validate_committed_attachments()?;
            }
            Ok(())
        };
        if let Err(error) = initialize() {
            if !database_existed || !alone {
                return Err(error);
            }
            log::warn!(
                "discarding incompatible Agent history index path={} error={error}",
                store.inner.paths.sqlite_database.display()
            );
            discard_sqlite(&store.inner.paths.sqlite_database)
                .map_err(AgentHistoryStoreError::Io)?;
            // The rebuild judges every session again.
            store.inner.projection_lag.lock().clear();
            store.initialize_database()?;
            store.catch_up_discovered_sessions()?;
            store.reconcile_session_locations()?;
            store.validate_committed_attachments()?;
        }
        store.record_quarantined_references();
        if alone {
            store.cleanup_all_unreferenced_attachments()?;
            drop(exclusive);
            *store.inner.lifetime.lock() =
                Some(FileLock::shared(&lifetime_path).map_err(io_error)?);
        }
        Ok(store)
    }
}
