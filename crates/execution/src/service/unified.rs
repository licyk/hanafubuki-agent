use super::*;

impl AgentExecutionService {
    pub fn wait_unified_execution(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        yield_time_ms: u64,
        max_output_bytes: Option<usize>,
        cancellation: &CancellationToken,
    ) -> Result<UnifiedExecResult, AgentExecutionError> {
        let started = Instant::now();
        let waited = self.wait(scope, execution_id, yield_time_ms, cancellation)?;
        self.unified_result(
            scope,
            execution_id,
            waited.completed,
            max_output_bytes,
            started,
        )
    }

    pub fn write_stdin(
        &self,
        scope: &AgentExecutionScope,
        request: WriteStdinRequest,
        cancellation: &CancellationToken,
    ) -> Result<UnifiedExecResult, AgentExecutionError> {
        let started = Instant::now();
        if request.chars.len() > 16 * 1024 {
            return Err(AgentExecutionError::InvalidRequest(
                "stdin input exceeds 16384 bytes".into(),
            ));
        }
        if !request.chars.is_empty() {
            let (receipt, io_mode) =
                self.receipt_and_io_mode_for_scope(scope, request.session_id)?;
            match (io_mode, request.chars.as_str()) {
                (ExecutorIoMode::Pty, chars) => self
                    .inner
                    .process
                    .write(&receipt, chars.as_bytes().to_vec(), false)
                    .map_err(AgentExecutionError::Process)?,
                (ExecutorIoMode::Pipe, "\u{3}") => self
                    .inner
                    .process
                    .interrupt(&receipt)
                    .map_err(AgentExecutionError::Process)?,
                (ExecutorIoMode::Pipe, _) => return Err(AgentExecutionError::NotInteractive),
            }
        }
        let waited = self.wait(
            scope,
            request.session_id,
            request.yield_time_ms,
            cancellation,
        )?;
        self.unified_result(
            scope,
            request.session_id,
            waited.completed,
            request.max_output_bytes,
            started,
        )
    }

    fn unified_result(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        completed: bool,
        max_output_bytes: Option<usize>,
        started: Instant,
    ) -> Result<UnifiedExecResult, AgentExecutionError> {
        let from_offset = {
            let registry = self.inner.registry.lock();
            let record = registry
                .records
                .get(&execution_id)
                .ok_or(AgentExecutionError::NotFound)?;
            if &record.scope != scope {
                return Err(AgentExecutionError::CrossScope);
            }
            record.unified_output_offset
        };
        let output = self.read_output(scope, execution_id, Some(from_offset), max_output_bytes)?;
        let mut text = output
            .chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>();
        let mut next_offset = output.next_offset;
        // A running command's next poll re-reads an unfinished trailing escape
        // sequence with the rest of it rather than printing it as text. The
        // held text is the exact bytes read when it decoded cleanly.
        if !completed {
            let complete = complete_prefix_len(&text);
            if !text[complete..].contains(char::REPLACEMENT_CHARACTER) {
                next_offset -= (text.len() - complete) as u64;
                text.truncate(complete);
            }
        }
        {
            let mut registry = self.inner.registry.lock();
            let record = registry
                .records
                .get_mut(&execution_id)
                .ok_or(AgentExecutionError::NotFound)?;
            if &record.scope != scope {
                return Err(AgentExecutionError::CrossScope);
            }
            record.unified_output_offset = next_offset;
        }
        let exit_code = output.exit_code;
        let truncated = output.truncated;
        if completed {
            self.await_observer_release(execution_id);
            if let Err(error) = self.dispose(scope, execution_id) {
                log::warn!(
                    "agent_diag stage=unified_exec.complete execution_id={} unified_exec_outcome=cleanup_retained unified_exec_session_live=false cleanup_obligation=true",
                    execution_id
                );
                return Err(error);
            }
        }
        log::debug!(
            "agent_diag stage=unified_exec.complete execution_id={} unified_exec_outcome={} unified_exec_session_live={} cleanup_obligation=false wall_time_ms={}",
            execution_id,
            if completed { "terminal" } else { "yielded" },
            !completed,
            started.elapsed().as_millis()
        );
        Ok(UnifiedExecResult {
            output: render_plain_text(text.as_bytes()),
            exit_code: completed.then_some(exit_code).flatten(),
            session_id: (!completed).then_some(execution_id.value()),
            wall_time_ms: started.elapsed().as_millis() as u64,
            truncated,
        })
    }
}
