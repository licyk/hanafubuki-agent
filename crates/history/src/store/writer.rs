use super::jsonl::{append_commit_to_file, read_writer_state};
use crate::paths::new_session_path;
use crate::store::*;
use hanafubuki_agent_foundation::background_worker::BackgroundWorker;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

const WRITER_CHANNEL_CAPACITY: usize = 64;
/// How often shutdown retries queueing a writer's stop while its queue is
/// full.
const SHUTDOWN_SEND_RETRY: Duration = Duration::from_millis(5);

/// Tells one session's writer from the writer that replaces it.
static NEXT_WRITER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(super) struct WriterHandle {
    pub(in crate::store) sender: mpsc::SyncSender<WriterCommand>,
    id: u64,
    /// Ends when the writer's thread does; a writer that panicked is
    /// replaced by the next command that needs one.
    worker: BackgroundWorker,
}

pub(in crate::store) enum WriterCommand {
    /// Acknowledged on the writer once the append is durable or failed.
    Append {
        commit: PendingHistoryCommit,
        ack: AgentHistoryAppendAck,
    },
    /// Reads attachments for a model request of the session, so the
    /// request's builder reads no file itself.
    LoadAttachments {
        attachments: Vec<hanafubuki_agent_kernel::AgentImageAttachmentRef>,
        ack: hanafubuki_agent_kernel::AgentImageAttachmentLoadAck,
    },
    Flush {
        ack: mpsc::SyncSender<Result<(), AgentHistoryStoreError>>,
    },
    Shutdown {
        ack: mpsc::SyncSender<Result<(), AgentHistoryStoreError>>,
    },
}

impl FileAgentHistoryStore {
    pub(in crate::store) fn session_gate(&self, session_id: AgentSessionId) -> Arc<Mutex<()>> {
        self.inner
            .writer_gates
            .lock()
            .entry(session_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// The session's writer, started when it has none. A writer whose
    /// thread has ended (it panicked) is withdrawn and a new one started, so
    /// one failed command does not stop the session's history. Callers hold
    /// the session gate.
    pub(in crate::store) fn ensure_writer(
        &self,
        session_id: AgentSessionId,
        requested_path: Option<PathBuf>,
    ) -> Result<WriterHandle, AgentHistoryStoreError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentHistoryStoreError::ShuttingDown);
        }
        let existing = self.inner.writers.lock().get(&session_id).cloned();
        if let Some(writer) = existing {
            if !writer.worker.has_exited() {
                return Ok(writer);
            }
            self.withdraw_writer(session_id, &writer);
        }
        let path = match requested_path {
            Some(path) => path,
            None => self
                .lookup_rollout_path(session_id)?
                .unwrap_or_else(|| new_session_path(&self.inner.paths, session_id)),
        };
        ensure_scoped_path(&path, &self.inner.paths.root)?;
        let (sender, receiver) = mpsc::sync_channel(WRITER_CHANNEL_CAPACITY);
        let store = self.clone();
        let worker = self
            .inner
            .workers
            .spawn_observed(
                format!("agent-history-writer-{session_id}"),
                move |cancellation| run_writer(store, session_id, path, receiver, cancellation),
            )
            .map_err(AgentHistoryStoreError::Io)?;
        let handle = WriterHandle {
            sender,
            id: NEXT_WRITER_ID.fetch_add(1, Ordering::Relaxed),
            worker,
        };
        self.inner.writers.lock().insert(session_id, handle.clone());
        Ok(handle)
    }

    /// Forgets `writer` if it is still the session's, so the next command
    /// starts a new one.
    fn withdraw_writer(&self, session_id: AgentSessionId, writer: &WriterHandle) {
        let mut writers = self.inner.writers.lock();
        if writers
            .get(&session_id)
            .is_some_and(|current| current.id == writer.id)
        {
            writers.remove(&session_id);
            log::warn!(
                "Agent history writer for session {session_id} stopped; the next write starts a new one"
            );
        }
    }

    /// Sends `command` to the session's writer. A writer found stopped by the
    /// send is replaced and the command sent once more; a command no writer
    /// takes is handed back with the error. Callers hold the session gate.
    fn send_to_writer(
        &self,
        session_id: AgentSessionId,
        requested_path: Option<PathBuf>,
        command: WriterCommand,
    ) -> Result<(), (AgentHistoryStoreError, WriterCommand)> {
        let writer = match self.ensure_writer(session_id, requested_path.clone()) {
            Ok(writer) => writer,
            Err(error) => return Err((error, command)),
        };
        let Err(mpsc::SendError(command)) = writer.sender.send(command) else {
            return Ok(());
        };
        self.withdraw_writer(session_id, &writer);
        let writer = match self.ensure_writer(session_id, requested_path) {
            Ok(writer) => writer,
            Err(error) => return Err((error, command)),
        };
        writer
            .sender
            .send(command)
            .map_err(|mpsc::SendError(command)| (AgentHistoryStoreError::ShuttingDown, command))
    }

    /// Appends `commit` on the session's writer and waits for it to
    /// acknowledge it. Callers hold the session gate.
    pub(in crate::store) fn send_append(
        &self,
        session_id: AgentSessionId,
        requested_path: Option<PathBuf>,
        commit: PendingHistoryCommit,
    ) -> Result<(), AgentHistoryStoreError> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        let ack = AgentHistoryAppendAck::new(move |outcome| {
            let _ = ack_tx.send(outcome);
            None
        });
        if let Err((error, command)) = self.send_to_writer(
            session_id,
            requested_path,
            WriterCommand::Append { commit, ack },
        ) {
            fail_command(command, error);
        }
        ack_rx
            .recv()
            .map_err(|_| AgentHistoryStoreError::ShuttingDown)?
    }

    /// Queues `commit` on the session's writer without waiting for it; the
    /// writer acknowledges it through `ack`. The session gate is held only
    /// to start the writer and queue the append: the writer's channel keeps
    /// the session's appends in order, and a lifecycle move or deletion that
    /// takes the gate later stops the writer behind them.
    pub(in crate::store) fn enqueue_append(
        &self,
        session_id: AgentSessionId,
        commit: PendingHistoryCommit,
        ack: AgentHistoryAppendAck,
    ) {
        let gate = self.session_gate(session_id);
        let _guard = gate.lock();
        if let Err((error, command)) =
            self.send_to_writer(session_id, None, WriterCommand::Append { commit, ack })
        {
            fail_command(command, error);
        }
    }

    /// Queues an attachment load on the session's writer, like
    /// `enqueue_append`; the writer reads the attachments and acknowledges.
    pub(in crate::store) fn enqueue_attachment_load(
        &self,
        session_id: AgentSessionId,
        attachments: Vec<hanafubuki_agent_kernel::AgentImageAttachmentRef>,
        ack: hanafubuki_agent_kernel::AgentImageAttachmentLoadAck,
    ) {
        let writer = {
            let gate = self.session_gate(session_id);
            let _guard = gate.lock();
            self.ensure_writer(session_id, None)
        };
        let writer = match writer {
            Ok(writer) => writer,
            Err(error) => return ack.fail(&attachments, error),
        };
        if let Err(mpsc::SendError(command)) = writer
            .sender
            .send(WriterCommand::LoadAttachments { attachments, ack })
        {
            // Sent without the gate: the next command that holds it starts
            // a new writer.
            fail_command(command, AgentHistoryStoreError::ShuttingDown);
        }
    }

    pub(in crate::store) fn create_session_inner(
        &self,
        commit: AgentConversationCommit,
    ) -> Result<(), AgentHistoryStoreError> {
        let session_id = commit.runtime.id;
        let gate = self.session_gate(session_id);
        let _guard = gate.lock();
        if self.lookup_rollout_path(session_id)?.is_some() {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "session {session_id} already exists"
            )));
        }
        let path = new_session_path(&self.inner.paths, session_id);
        self.send_append(
            session_id,
            Some(path),
            PendingHistoryCommit::checkpoint(commit),
        )
    }

    pub(in crate::store) fn append_commit_inner(
        &self,
        commit: AgentConversationCommit,
    ) -> Result<(), AgentHistoryStoreError> {
        let session_id = commit.runtime.id;
        let gate = self.session_gate(session_id);
        let _guard = gate.lock();
        self.send_append(session_id, None, PendingHistoryCommit::checkpoint(commit))
    }

    pub(in crate::store) fn append_delta_inner(
        &self,
        delta: AgentConversationDelta,
    ) -> Result<(), AgentHistoryStoreError> {
        let session_id = delta.session_id;
        let gate = self.session_gate(session_id);
        let _guard = gate.lock();
        self.send_append(session_id, None, PendingHistoryCommit::delta(delta))
    }

    pub(in crate::store) fn create_fork_inner(
        &self,
        source_session_id: AgentSessionId,
        before_turn_id: AgentTurnId,
        mut commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        let history_base = self.lookup_before_turn_base(source_session_id, before_turn_id)?;
        commit.runtime.history_base = Some(history_base);
        self.create_session_inner(commit)?;
        Ok(history_base)
    }

    pub(in crate::store) fn create_fork_at_head_inner(
        &self,
        source_session_id: AgentSessionId,
        source_revision: AgentRevision,
        mut commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        let history_base = self.lookup_head_base(source_session_id, source_revision)?;
        commit.runtime.history_base = Some(history_base);
        self.create_session_inner(commit)?;
        Ok(history_base)
    }

    pub(in crate::store) fn flush_session_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        let writer = self
            .inner
            .writers
            .lock()
            .get(&session_id)
            .cloned()
            .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        writer
            .sender
            .send(WriterCommand::Flush { ack: ack_tx })
            .map_err(|_| AgentHistoryStoreError::ShuttingDown)?;
        ack_rx
            .recv()
            .map_err(|_| AgentHistoryStoreError::ShuttingDown)?
    }

    pub(in crate::store) fn shutdown_session_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        let writer = self
            .inner
            .writers
            .lock()
            .remove(&session_id)
            .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        writer
            .sender
            .send(WriterCommand::Shutdown { ack: ack_tx })
            .map_err(|_| AgentHistoryStoreError::ShuttingDown)?;
        ack_rx
            .recv()
            .map_err(|_| AgentHistoryStoreError::ShuttingDown)?
    }

    /// Stops the session's writer behind the commands queued before it,
    /// waiting until `deadline`: for room in its queue, then for its
    /// acknowledgement. A writer still busy then (a stalled disk) keeps
    /// running detached and is reported; it stops at its next command or
    /// with the worker group.
    fn shutdown_session_until(
        &self,
        session_id: AgentSessionId,
        deadline: Instant,
    ) -> Result<(), AgentHistoryStoreError> {
        let writer = self
            .inner
            .writers
            .lock()
            .remove(&session_id)
            .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        let mut command = WriterCommand::Shutdown { ack: ack_tx };
        loop {
            match writer.sender.try_send(command) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(AgentHistoryStoreError::ShuttingDown)
                }
                Err(mpsc::TrySendError::Full(unsent)) => {
                    if Instant::now() >= deadline {
                        return Err(AgentHistoryStoreError::WriterDidNotStop { session_id });
                    }
                    command = unsent;
                    std::thread::sleep(SHUTDOWN_SEND_RETRY);
                }
            }
        }
        match ack_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err(AgentHistoryStoreError::WriterDidNotStop { session_id })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(AgentHistoryStoreError::ShuttingDown),
        }
    }

    /// Makes the next append panic on its writer, and reports whether the
    /// session's current writer has ended.
    #[cfg(test)]
    pub(in crate::store) fn test_panic_next_append(&self) {
        self.inner.panic_next_append.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(in crate::store) fn test_writer_exited(&self, session_id: AgentSessionId) -> bool {
        self.inner
            .writers
            .lock()
            .get(&session_id)
            .is_some_and(|writer| writer.worker.has_exited())
    }

    pub(in crate::store) fn discard_live_session_inner(&self, session_id: AgentSessionId) {
        self.inner.writers.lock().remove(&session_id);
    }

    pub(in crate::store) fn shutdown_inner(
        &self,
        deadline: Instant,
    ) -> Vec<AgentHistoryStoreError> {
        self.stop_accepting();
        let mut session_ids = self
            .inner
            .writers
            .lock()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        session_ids.sort_unstable();
        session_ids
            .into_iter()
            .filter_map(|session_id| {
                let stopped = self.shutdown_session_until(session_id, deadline);
                if let Err(error) = &stopped {
                    log::warn!("Agent history shutdown of session {session_id}: {error}");
                }
                stopped.err()
            })
            .collect()
    }
}

/// Fails a command no writer took with `error`.
fn fail_command(command: WriterCommand, error: AgentHistoryStoreError) {
    match command {
        WriterCommand::Append { ack, .. } => ack.fail(error),
        WriterCommand::LoadAttachments { attachments, ack } => ack.fail(&attachments, error),
        WriterCommand::Flush { ack } | WriterCommand::Shutdown { ack } => {
            let _ = ack.send(Err(error));
        }
    }
}

fn run_writer(
    store: FileAgentHistoryStore,
    session_id: AgentSessionId,
    path: PathBuf,
    receiver: mpsc::Receiver<WriterCommand>,
    cancellation: hanafubuki_agent_foundation::cancellation::CancellationToken,
) {
    let mut writer = SessionWriter::open(store, session_id, path);
    while !cancellation.is_cancelled() {
        let command = match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match command {
            WriterCommand::Append { commit, ack } => {
                #[cfg(test)]
                if writer
                    .store
                    .inner
                    .panic_next_append
                    .swap(false, Ordering::SeqCst)
                {
                    panic!("injected Agent history writer panic");
                }
                let appended = writer.append(commit);
                // A follow-up the acknowledgement hands back is appended
                // here, before the next queued command.
                ack.acknowledge(appended, |commit| {
                    writer.append(PendingHistoryCommit::checkpoint(commit))
                });
            }
            WriterCommand::LoadAttachments { attachments, ack } => {
                ack.acknowledge(
                    attachments
                        .iter()
                        .map(|attachment| writer.store.load_image_attachment_inner(attachment))
                        .collect(),
                );
            }
            WriterCommand::Flush { ack } => {
                let result = writer
                    .file
                    .as_mut()
                    .map_or(Ok(()), |file| file.sync_data().map_err(io_error));
                let _ = ack.send(result);
            }
            WriterCommand::Shutdown { ack } => {
                let _ = ack.send(writer.shut_down());
                return;
            }
        }
    }
    if let Some(file) = writer.file.as_mut() {
        let _ = file.sync_all();
    }
}

/// One session's JSONL file, as its writer appends to it.
struct SessionWriter {
    store: FileAgentHistoryStore,
    session_id: AgentSessionId,
    path: PathBuf,
    file: Option<File>,
    repair_registered: bool,
    next_ordinal: u64,
    last_revision: Option<u64>,
    startup_error: Option<String>,
}

impl SessionWriter {
    fn open(store: FileAgentHistoryStore, session_id: AgentSessionId, path: PathBuf) -> Self {
        let (next_ordinal, last_revision, startup_error) = match read_writer_state(&path) {
            Ok((next, revision)) => (next, revision, None),
            Err(error) => (0, None, Some(error.to_string())),
        };
        Self {
            store,
            session_id,
            path,
            file: None,
            repair_registered: false,
            next_ordinal,
            last_revision,
            startup_error,
        }
    }

    /// Appends `commit` to the file, then updates the projection.
    fn append(&mut self, commit: PendingHistoryCommit) -> Result<(), AgentHistoryStoreError> {
        let (store, session_id) = (&self.store, self.session_id);
        let pending_result = if self.repair_registered {
            Ok(())
        } else {
            let result =
                store.mark_projection_pending(session_id, &self.path, commit.session_revision);
            if result.is_ok() {
                self.repair_registered = true;
            }
            result
        };
        let canonical_result = if let Err(error) = pending_result {
            Err(error)
        } else if let Some(error) = &self.startup_error {
            Err(AgentHistoryStoreError::CorruptHistory(error.clone()))
        } else {
            append_commit_to_file(
                &self.path,
                &mut self.file,
                &mut self.next_ordinal,
                &mut self.last_revision,
                commit,
            )
        };
        if canonical_result.is_ok() {
            match store.materialize_path(session_id, &self.path) {
                Ok(()) => store.clear_append_lag(session_id),
                Err(error) => {
                    log::warn!(
                        "Agent history projection lags durable JSONL for session {session_id}: {error}"
                    );
                    store.record_append_lag(session_id, error.to_string());
                }
            }
        } else {
            // A failed write may have left an incomplete suffix. Drop the
            // handle so the next append reopens the file, repairs only the
            // non-newline tail, and derives ordinal/revision from the
            // canonical prefix again.
            self.file = None;
            if let Ok((ordinal, revision)) = read_writer_state(&self.path) {
                self.next_ordinal = ordinal;
                self.last_revision = revision;
            }
        }
        canonical_result
    }

    /// Syncs the file and projects what an append left unprojected: once
    /// the index has caught up, the pending repair and an append's lag are
    /// over, so the session is served again without a restart.
    fn shut_down(&mut self) -> Result<(), AgentHistoryStoreError> {
        let result = self
            .file
            .as_mut()
            .map_or(Ok(()), |file| file.sync_all().map_err(io_error));
        let result = result.and_then(|()| self.store.materialize_path(self.session_id, &self.path));
        result.and_then(|()| self.store.clear_projection_pending(self.session_id))?;
        self.store.clear_append_lag(self.session_id);
        Ok(())
    }
}
