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
                    .map_err(|error| self.port_error(request.session_id, error))?,
                (ExecutorIoMode::Pipe, "\u{3}") => self
                    .inner
                    .process
                    .interrupt(&receipt)
                    .map_err(|error| self.port_error(request.session_id, error))?,
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
        let bound = read_bound(max_output_bytes)?;
        let (receipt, from_offset, io_mode, screen) = {
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
            record.updated_at_ms = now_millis();
            (
                receipt,
                record.unified_output_offset,
                record.io_mode,
                record.screen.take(),
            )
        };
        // Everything not read yet, so the bound can keep its head and tail.
        let output = self
            .inner
            .process
            .read(&receipt, Some(from_offset), None)
            .map_err(|error| self.port_error(execution_id, error))?;
        let bytes = output
            .chunks
            .iter()
            .flat_map(|chunk| chunk.data.iter().copied())
            .collect::<Vec<_>>();
        let (text, next_offset, omitted, screen) = if io_mode.uses_pty() {
            let mut screen = screen.unwrap_or_else(|| {
                TerminalScreen::new(output.terminal_size.unwrap_or(TerminalSize::DEFAULT))
            });
            let (text, omitted) = render_window(
                &output_window(&bytes, from_offset, output.start_offset, bound),
                |bytes, offset, after_gap| {
                    if after_gap {
                        screen = TerminalScreen::new(TerminalSize::DEFAULT);
                    }
                    screen.push_sized(bytes, offset, output.terminal_size, &output.resizes);
                    screen.take().text
                },
            );
            (text, output.next_offset, omitted, Some(screen))
        } else {
            // A running command's next poll re-reads an unfinished trailing
            // escape sequence with the rest of it rather than printing it as
            // text.
            let complete = if completed {
                bytes.len()
            } else {
                complete_prefix_len(&bytes)
            };
            let (text, omitted) = render_window(
                &output_window(&bytes[..complete], from_offset, output.start_offset, bound),
                |bytes, _, _| render_plain_text(bytes),
            );
            let held = (bytes.len() - complete) as u64;
            (text, output.next_offset - held, omitted, None)
        };
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
            record.screen = screen;
        }
        let exit_code = output.exit_code;
        let truncated = output.truncated || omitted;
        if completed {
            self.await_observer_release(execution_id);
            if let Err(error) = self.dispose(scope, execution_id) {
                if matches!(error, AgentExecutionError::NotFound) {
                    return Err(error);
                }
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
            output: text,
            exit_code: completed.then_some(exit_code).flatten(),
            session_id: (!completed).then_some(execution_id.value()),
            wall_time_ms: started.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

/// Render each text part of a poll's `window` with `render` (given the part's
/// bytes, its offset, and whether a gap precedes it), and each gap as its
/// marker on a line of its own; returns the text and whether it has a gap.
fn render_window(
    window: &[WindowPart<'_>],
    mut render: impl FnMut(&[u8], u64, bool) -> String,
) -> (String, bool) {
    let mut text = String::new();
    let mut after_gap = false;
    for part in window {
        match part {
            WindowPart::Text { bytes, offset } => {
                text.push_str(&render(bytes, *offset, after_gap));
                after_gap = false;
            }
            WindowPart::Omitted(omitted) => {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&omission_marker(*omitted));
                text.push('\n');
                after_gap = true;
            }
        }
    }
    let omitted = window
        .iter()
        .any(|part| matches!(part, WindowPart::Omitted(_)));
    (text, omitted)
}
