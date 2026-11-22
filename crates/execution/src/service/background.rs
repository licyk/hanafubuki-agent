use super::*;

impl AgentExecutionService {
    pub fn list_background_terminals(
        &self,
        scope: &AgentExecutionScope,
    ) -> Result<Vec<AgentBackgroundTerminalSnapshot>, AgentExecutionError> {
        let entries = {
            let registry = self.inner.registry.lock();
            registry
                .records
                .iter()
                .filter(|(_, record)| {
                    &record.scope == scope
                        && record.kind == AgentExecutionKind::UnifiedExec
                        && record.receipt.is_some()
                        && !record.disposing
                })
                .map(|(execution_id, record)| {
                    (
                        *execution_id,
                        record.command_display.clone().unwrap_or_default(),
                        record.cwd_display.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut terminals = Vec::with_capacity(entries.len());
        for (execution_id, command, cwd) in entries {
            match self.background_terminal_snapshot(scope, execution_id, command, cwd) {
                Ok(terminal) => terminals.push(terminal),
                // A close can complete after the list of IDs was collected.
                Err(AgentExecutionError::NotFound) => continue,
                Err(error) => return Err(error),
            }
        }
        terminals.sort_by_key(|terminal| terminal.execution_id.value());
        Ok(terminals)
    }

    fn background_terminal_snapshot(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        command: String,
        cwd: String,
    ) -> Result<AgentBackgroundTerminalSnapshot, AgentExecutionError> {
        let snapshot = self.snapshot(scope, execution_id)?;
        let from_offset = snapshot
            .output_offset
            .saturating_sub(BACKGROUND_OUTPUT_BYTES as u64);
        let output = self.read_output(
            scope,
            execution_id,
            Some(from_offset),
            Some(BACKGROUND_OUTPUT_BYTES),
        )?;
        let recent_output = recent_output_lines(&output);
        Ok(AgentBackgroundTerminalSnapshot {
            execution_id,
            generation: snapshot.generation,
            command,
            cwd,
            status: snapshot.status,
            exit_code: snapshot.exit_code,
            created_at_ms: snapshot.created_at_ms,
            updated_at_ms: snapshot.updated_at_ms,
            cleanup_required: snapshot.cleanup_required,
            recent_output,
        })
    }

    pub fn stop_background_terminals_confirmed(
        &self,
        scope: &AgentExecutionScope,
    ) -> AgentBackgroundTerminalStopOutcome {
        self.inner
            .shell_snapshots
            .lock()
            .remove(&scope.session_id());
        let identities = {
            let registry = self.inner.registry.lock();
            registry
                .records
                .iter()
                .filter(|(_, record)| {
                    &record.scope == scope
                        && record.kind == AgentExecutionKind::UnifiedExec
                        && record.receipt.is_some()
                })
                .map(|(execution_id, record)| (*execution_id, record.generation))
                .collect::<Vec<_>>()
        };
        let mut outcome = AgentBackgroundTerminalStopOutcome {
            disposed: Vec::new(),
            retained: Vec::new(),
        };
        for (execution_id, generation) in identities {
            let result = self
                .ensure_generation(scope, execution_id, generation)
                .and_then(|()| self.stop_confirmed(scope, execution_id));
            match result {
                Ok(disposed) => outcome.disposed.push(disposed),
                Err(AgentExecutionError::NotFound) => {
                    outcome.disposed.push(ConfirmedAgentExecutionDisposal {
                        execution_id,
                        generation,
                    });
                }
                Err(_) => outcome.retained.push(AgentBackgroundTerminalStopFailure {
                    execution_id,
                    generation,
                    message: "Background terminal cleanup could not be confirmed".to_string(),
                }),
            }
        }
        outcome
    }
}
