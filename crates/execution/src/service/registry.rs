use super::*;

pub(super) struct ExecutionRegistry {
    pub(super) next_execution_id: u64,
    pub(super) records: HashMap<AgentExecutionId, ExecutionRecord>,
}

pub(super) struct ExecutionRecord {
    pub(super) scope: AgentExecutionScope,
    pub(super) kind: AgentExecutionKind,
    pub(super) command_display: Option<String>,
    pub(super) cwd_display: String,
    pub(super) receipt: Option<ProcessReceipt>,
    pub(super) process_id: String,
    pub(super) created_at_ms: u128,
    pub(super) updated_at_ms: u128,
    pub(super) cleanup_required: bool,
    pub(super) last_error: Option<String>,
    pub(super) generation: AgentExecutionGeneration,
    pub(super) io_mode: ExecutorIoMode,
    pub(super) interactive: bool,
    pub(super) unified_output_offset: u64,
    /// A PTY execution's screen, as `exec_command` and `write_stdin` polls
    /// last read it.
    pub(super) screen: Option<TerminalScreen>,
    /// The style a pipe execution's output was in where the last poll
    /// ended, which the next one starts in.
    pub(super) pen: TextStyle,
    /// A progress reporter is streaming this execution's output.
    pub(super) observed: bool,
    /// One caller is disposing this execution; to every other caller it is
    /// already gone (`NotFound`).
    pub(super) disposing: bool,
}

impl AgentExecutionService {
    #[cfg(any(test, feature = "test-support"))]
    pub fn registry_count(&self) -> usize {
        self.inner.registry.lock().records.len()
    }

    pub(super) fn reserve_record(
        &self,
        scope: AgentExecutionScope,
        kind: AgentExecutionKind,
        interactive: bool,
        command_display: Option<String>,
        cwd_display: String,
    ) -> Result<(AgentExecutionId, String), AgentExecutionError> {
        let mut registry = self.inner.registry.lock();
        let execution_id = AgentExecutionId::new(registry.next_execution_id);
        registry.next_execution_id = registry
            .next_execution_id
            .checked_add(1)
            .ok_or_else(|| AgentExecutionError::Process("execution id exhausted".to_string()))?;
        let process_id = format!("agent-command-{}", execution_id.value());
        let now = now_millis();
        registry.records.insert(
            execution_id,
            ExecutionRecord {
                scope,
                kind,
                command_display,
                cwd_display,
                receipt: None,
                process_id: process_id.clone(),
                created_at_ms: now,
                updated_at_ms: now,
                cleanup_required: false,
                last_error: None,
                generation: AgentExecutionGeneration::initial(),
                io_mode: ExecutorIoMode::Pipe,
                interactive,
                unified_output_offset: 0,
                screen: None,
                pen: TextStyle::default(),
                observed: false,
                disposing: false,
            },
        );
        Ok((execution_id, process_id))
    }

    pub(super) fn commit_receipt(
        &self,
        execution_id: AgentExecutionId,
        receipt: ProcessReceipt,
        info: &ExecutorInfo,
    ) -> Result<(), AgentExecutionError> {
        let mut registry = self.inner.registry.lock();
        let record = registry
            .records
            .get_mut(&execution_id)
            .ok_or(AgentExecutionError::NotFound)?;
        let process_id_matches =
            record.process_id == receipt.process_id() && info.executor_id == record.process_id;
        record.receipt = Some(receipt);
        record.io_mode = info.io_mode;
        record.updated_at_ms = now_millis();
        if !process_id_matches {
            record.cleanup_required = true;
            record.last_error =
                Some("Executor committed a mismatched process identity".to_string());
            return Err(AgentExecutionError::Process(
                "Executor committed a mismatched process identity".to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn remove_uncommitted(&self, execution_id: AgentExecutionId) {
        let mut registry = self.inner.registry.lock();
        if registry
            .records
            .get(&execution_id)
            .is_some_and(|record| record.receipt.is_none())
        {
            registry.records.remove(&execution_id);
        }
    }

    pub(super) fn receipt_for_scope(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
    ) -> Result<ProcessReceipt, AgentExecutionError> {
        let registry = self.inner.registry.lock();
        let record = registry
            .records
            .get(&execution_id)
            .filter(|record| !record.disposing)
            .ok_or(AgentExecutionError::NotFound)?;
        if &record.scope != scope {
            return Err(AgentExecutionError::CrossScope);
        }
        record.receipt.clone().ok_or(AgentExecutionError::NotFound)
    }

    pub(super) fn receipt_and_io_mode_for_scope(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
    ) -> Result<(ProcessReceipt, ExecutorIoMode), AgentExecutionError> {
        let registry = self.inner.registry.lock();
        let record = registry
            .records
            .get(&execution_id)
            .filter(|record| !record.disposing)
            .ok_or(AgentExecutionError::NotFound)?;
        if &record.scope != scope {
            return Err(AgentExecutionError::CrossScope);
        }
        Ok((
            record
                .receipt
                .clone()
                .ok_or(AgentExecutionError::NotFound)?,
            record.io_mode,
        ))
    }

    pub(super) fn receipt_for_generation(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        generation: AgentExecutionGeneration,
    ) -> Result<ProcessReceipt, AgentExecutionError> {
        let registry = self.inner.registry.lock();
        let record = registry
            .records
            .get(&execution_id)
            .filter(|record| !record.disposing)
            .ok_or(AgentExecutionError::NotFound)?;
        if &record.scope != scope {
            return Err(AgentExecutionError::CrossScope);
        }
        if record.generation != generation {
            return Err(AgentExecutionError::StaleGeneration);
        }
        if !record.interactive {
            return Err(AgentExecutionError::NotInteractive);
        }
        record.receipt.clone().ok_or(AgentExecutionError::NotFound)
    }

    pub(super) fn scope(
        &self,
        execution_id: AgentExecutionId,
    ) -> Result<AgentExecutionScope, AgentExecutionError> {
        self.inner
            .registry
            .lock()
            .records
            .get(&execution_id)
            .map(|record| record.scope.clone())
            .ok_or(AgentExecutionError::NotFound)
    }

    pub(super) fn snapshot_by_id(
        &self,
        execution_id: AgentExecutionId,
    ) -> Result<AgentExecutionSnapshot, AgentExecutionError> {
        let scope = self.scope(execution_id)?;
        self.snapshot(&scope, execution_id)
    }

    pub(super) fn update_and_snapshot(
        &self,
        execution_id: AgentExecutionId,
        info: &ExecutorInfo,
        error: Option<String>,
    ) -> Result<AgentExecutionSnapshot, AgentExecutionError> {
        let mut registry = self.inner.registry.lock();
        let record = registry
            .records
            .get_mut(&execution_id)
            .ok_or(AgentExecutionError::NotFound)?;
        record.updated_at_ms = now_millis();
        if let Some(error) = error {
            record.cleanup_required = true;
            record.last_error = Some(error);
        }
        Ok(AgentExecutionSnapshot {
            execution_id,
            generation: record.generation,
            kind: record.kind,
            status: info.status.into(),
            exit_code: info.exit_code,
            output_offset: info.output_offset,
            created_at_ms: record.created_at_ms,
            updated_at_ms: record.updated_at_ms,
            cleanup_required: record.cleanup_required,
            interactive: record.interactive,
            last_error: record.last_error.clone(),
        })
    }

    /// Claims the execution's disposal for this caller and returns its
    /// receipt; a concurrent caller then finds it gone.
    pub(super) fn claim_disposal(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
    ) -> Result<ProcessReceipt, AgentExecutionError> {
        let mut registry = self.inner.registry.lock();
        let record = registry
            .records
            .get_mut(&execution_id)
            .filter(|record| !record.disposing)
            .ok_or(AgentExecutionError::NotFound)?;
        if &record.scope != scope {
            return Err(AgentExecutionError::CrossScope);
        }
        let receipt = record
            .receipt
            .clone()
            .ok_or(AgentExecutionError::NotFound)?;
        record.disposing = true;
        Ok(receipt)
    }

    /// A failed disposal ends its claim: the execution stays, owing cleanup,
    /// for anyone to retry.
    pub(super) fn release_failed_disposal(
        &self,
        execution_id: AgentExecutionId,
        error: String,
    ) -> AgentExecutionError {
        if let Some(record) = self.inner.registry.lock().records.get_mut(&execution_id) {
            record.disposing = false;
        }
        self.retain_cleanup_error(execution_id, error)
    }

    pub(super) fn retain_cleanup_error(
        &self,
        execution_id: AgentExecutionId,
        error: String,
    ) -> AgentExecutionError {
        let error = bound_utf8(&error, MAX_EXECUTION_DIAGNOSTIC_BYTES);
        if let Some(record) = self.inner.registry.lock().records.get_mut(&execution_id) {
            record.cleanup_required = true;
            record.last_error = Some(error.clone());
            record.updated_at_ms = now_millis();
        }
        AgentExecutionError::CleanupRetained {
            execution_id,
            message: error,
        }
    }

    /// The error for a failed port call. A caller that fetched the receipt
    /// before another caller disposed the execution, or began to, finds the
    /// process gone: that is `NotFound`, not a process failure.
    pub(super) fn port_error(
        &self,
        execution_id: AgentExecutionId,
        error: String,
    ) -> AgentExecutionError {
        if self.is_live(execution_id) {
            AgentExecutionError::Process(error)
        } else {
            AgentExecutionError::NotFound
        }
    }

    /// Why the execution, which this caller found held by another, is not
    /// gone yet, as a diagnostic says it; `None` once it is gone.
    pub(super) fn held_elsewhere(&self, execution_id: AgentExecutionId) -> Option<String> {
        let registry = self.inner.registry.lock();
        let record = registry.records.get(&execution_id)?;
        let held = if record.disposing {
            "another caller was still confirming its end"
        } else if record.receipt.is_none() {
            "its process was still starting"
        } else {
            "it was still registered"
        };
        Some(match &record.last_error {
            Some(error) => format!("{held} (last error: {error})"),
            None => held.to_string(),
        })
    }

    pub(super) fn is_live(&self, execution_id: AgentExecutionId) -> bool {
        self.inner
            .registry
            .lock()
            .records
            .get(&execution_id)
            .is_some_and(|record| !record.disposing)
    }
}

pub(super) fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
