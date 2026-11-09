use super::*;

impl AgentExecutionService {
    pub fn snapshot(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
    ) -> Result<AgentExecutionSnapshot, AgentExecutionError> {
        let receipt = self.receipt_for_scope(scope, execution_id)?;
        let info = self
            .inner
            .process
            .info(&receipt)
            .map_err(|error| self.port_error(execution_id, error))?;
        self.update_and_snapshot(execution_id, &info, None)
    }

    pub fn read_output(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        from_offset: Option<u64>,
        max_bytes: Option<usize>,
    ) -> Result<AgentExecutionOutput, AgentExecutionError> {
        let max_bytes = max_bytes.unwrap_or(64 * 1024);
        if max_bytes == 0 || max_bytes > MAX_OUTPUT_READ_BYTES {
            return Err(AgentExecutionError::InvalidRequest(format!(
                "read size must be between 1 and {MAX_OUTPUT_READ_BYTES} bytes"
            )));
        }
        let receipt = self.receipt_for_scope(scope, execution_id)?;
        let output = self
            .inner
            .process
            .read(&receipt, from_offset, Some(max_bytes))
            .map_err(|error| self.port_error(execution_id, error))?;
        let now = now_millis();
        if let Some(record) = self.inner.registry.lock().records.get_mut(&execution_id) {
            record.updated_at_ms = now;
        }
        Ok(AgentExecutionOutput {
            execution_id,
            status: output.status.into(),
            exit_code: output.exit_code,
            chunks: output
                .chunks
                .into_iter()
                .map(|chunk| AgentExecutionOutputChunk {
                    stream: chunk.stream.into(),
                    text: String::from_utf8_lossy(&chunk.data).into_owned(),
                })
                .collect(),
            start_offset: output.start_offset,
            next_offset: output.next_offset,
            truncated: output.truncated,
        })
    }

    pub fn wait(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        wait_ms: u64,
        cancellation: &CancellationToken,
    ) -> Result<AgentExecutionWaitOutcome, AgentExecutionError> {
        let wait_started = Instant::now();
        log::debug!(
            "agent_diag stage=execution.wait.begin execution_id={execution_id} wait_ms={wait_ms}"
        );
        if wait_ms == 0 || wait_ms > MAX_WAIT_MS {
            return Err(AgentExecutionError::InvalidRequest(format!(
                "wait must be between 1 and {MAX_WAIT_MS} ms"
            )));
        }
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            if cancellation.is_cancelled() {
                log::debug!(
                    "agent_diag stage=execution.wait.end execution_id={execution_id} outcome=cancelled elapsed_ms={}",
                    wait_started.elapsed().as_millis()
                );
                return Err(AgentExecutionError::Cancelled);
            }
            let snapshot = self.snapshot(scope, execution_id)?;
            if snapshot.status.is_terminal() {
                log::debug!(
                    "agent_diag stage=execution.wait.end execution_id={execution_id} outcome=terminal elapsed_ms={}",
                    wait_started.elapsed().as_millis()
                );
                return Ok(AgentExecutionWaitOutcome {
                    snapshot,
                    completed: true,
                });
            }
            if Instant::now() >= deadline {
                log::debug!(
                    "agent_diag stage=execution.wait.end execution_id={execution_id} outcome=deadline elapsed_ms={}",
                    wait_started.elapsed().as_millis()
                );
                return Ok(AgentExecutionWaitOutcome {
                    snapshot,
                    completed: false,
                });
            }
            if cancellation
                .wait_timeout(WAIT_POLL.min(deadline.saturating_duration_since(Instant::now())))
            {
                log::debug!(
                    "agent_diag stage=execution.wait.end execution_id={execution_id} outcome=cancelled elapsed_ms={}",
                    wait_started.elapsed().as_millis()
                );
                return Err(AgentExecutionError::Cancelled);
            }
        }
    }
}

pub(super) fn recent_output_lines(output: &AgentExecutionOutput) -> Vec<String> {
    let combined = output
        .chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>();
    let mut lines = render_plain_text(combined.as_bytes())
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| bound_utf8(line, BACKGROUND_OUTPUT_LINE_BYTES))
        .collect::<Vec<_>>();
    if lines.len() > BACKGROUND_OUTPUT_LINES {
        lines.drain(..lines.len() - BACKGROUND_OUTPUT_LINES);
    }
    lines
}

pub(super) fn bound_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}
