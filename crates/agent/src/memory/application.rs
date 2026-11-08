use super::*;

impl AgentMemoryApplication {
    pub(crate) fn new(
        config: AgentConfig,
        core: AgentService,
        repository: AgentMemoryRepository,
        history: Arc<FileAgentHistoryStore>,
        workers: BackgroundWorkerGroup,
    ) -> Result<Self, String> {
        for source in history
            .list_memory_source_states()
            .map_err(|error| format!("could not rebuild Agent memory source projection: {error}"))?
        {
            repository
                .sync_source_state(
                    &source.session_id.to_string(),
                    source.contribution.mode,
                    source.contribution.revision,
                    source.source_revision.value(),
                )
                .map_err(|error| {
                    format!("could not rebuild Agent memory source projection: {error}")
                })?;
        }
        let (work_sender, work_receiver) = mpsc::sync_channel(64);
        let application = Self {
            config,
            core,
            repository,
            work_sender,
            supervisor_gone: Arc::default(),
            #[cfg(test)]
            scan_signals: Arc::default(),
        };
        let supervisor = application.clone();
        workers
            .spawn("agent-memory-supervisor", move |cancellation| {
                supervisor::run_memory_supervisor(supervisor, history, work_receiver, cancellation)
            })
            .map_err(|error| format!("could not start the Agent memory supervisor: {error}"))?;
        application.signal_scan(None);
        Ok(application)
    }

    pub fn repository(&self) -> &AgentMemoryRepository {
        &self.repository
    }

    pub fn sync_session_contribution(
        &self,
        session_id: AgentSessionId,
        state: hanafubuki_agent_kernel::AgentMemoryContributionState,
        source_revision: u64,
    ) -> Result<(), String> {
        self.repository
            .sync_source_state(
                &session_id.to_string(),
                state.mode,
                state.revision,
                source_revision,
            )
            .map_err(|error| error.to_string())
    }

    pub fn status(&self) -> Result<AgentMemoryStatus, String> {
        let settings = self.config.settings()?;
        self.repository
            .status(settings.memory.enabled)
            .map_err(|error| error.to_string())
    }

    pub fn enabled_for_use(&self) -> Result<bool, String> {
        let memory = self.config.settings()?.memory;
        Ok(memory.enabled && memory.use_memories)
    }

    pub fn enabled_for_generation(&self) -> Result<bool, String> {
        let memory = self.config.settings()?.memory;
        Ok(memory.enabled && memory.generate_memories)
    }

    pub fn append_ad_hoc_note(
        &self,
        request: hanafubuki_agent_memory::AppendAdHocMemoryNote,
    ) -> Result<hanafubuki_agent_memory::AdHocMemoryNote, String> {
        let note = self
            .repository
            .append_ad_hoc_note(request)
            .map_err(|error| error.to_string())?;
        self.signal_scan(None);
        Ok(note)
    }

    pub fn reset(&self, expected_generation: u64) -> Result<u64, String> {
        let generation = self
            .repository
            .reset(expected_generation)
            .map_err(|error| error.to_string())?;
        self.signal_scan(None);
        Ok(generation)
    }

    pub fn signal_scan(&self, current_session_id: Option<AgentSessionId>) {
        #[cfg(test)]
        self.scan_signals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self
            .work_sender
            .try_send(MemoryWork::Scan(current_session_id))
        {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
            // The supervisor stops at shutdown, or when its worker panicked
            // (logged by its worker group); either way memory stops being
            // generated, which is worth one warning.
            Err(mpsc::TrySendError::Disconnected(_)) => {
                if !self
                    .supervisor_gone
                    .swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    log::warn!(
                        "Agent memory supervisor is not running (stopped by shutdown or ended by a panic); memory scans are not run until the application restarts"
                    );
                }
            }
        }
    }

    /// How many scans have been signalled, whether or not the queue took
    /// them.
    #[cfg(test)]
    pub(crate) fn test_scan_signals(&self) -> usize {
        self.scan_signals.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Queues a usage update for the memory sources a committed answer
    /// cited.
    pub fn record_citation_usage(
        &self,
        citations: &[AgentMemoryCitationSnapshot],
    ) -> Result<(), String> {
        let source_session_ids = citations
            .iter()
            .flat_map(|citation| citation.source_session_ids.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        for chunk in source_session_ids.chunks(hanafubuki_agent_memory::MAX_USAGE_SOURCES) {
            self.work_sender
                .try_send(MemoryWork::RecordCitationUsage {
                    source_session_ids: chunk.to_vec(),
                })
                .map_err(|error| format!("Agent memory usage queue unavailable: {error}"))?;
        }
        Ok(())
    }

    pub fn revoke_session(&self, session_id: AgentSessionId) -> Result<(), String> {
        self.repository
            .revoke_source(&session_id.to_string())
            .map_err(|error| error.to_string())?;
        self.signal_scan(None);
        Ok(())
    }
}
