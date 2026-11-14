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
        let (receipt, from_offset, io_mode, screen, pen) = {
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
                record.pen,
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
        let (rendered, next_offset, screen, pen) = if io_mode.uses_pty() {
            let mut screen = screen.unwrap_or_else(|| {
                TerminalScreen::new(output.terminal_size.unwrap_or(TerminalSize::DEFAULT))
            });
            let rendered = render_window(
                &output_window(&bytes, from_offset, output.start_offset, bound),
                |bytes, offset, after_gap| {
                    if after_gap {
                        screen = TerminalScreen::new(TerminalSize::DEFAULT);
                    }
                    screen.push_sized(bytes, offset, output.terminal_size, &output.resizes);
                    let taken = screen.take();
                    (taken.text, taken.styles)
                },
            );
            (rendered, output.next_offset, Some(screen), pen)
        } else {
            // A running command's next poll re-reads an unfinished trailing
            // escape sequence with the rest of it rather than printing it as
            // text.
            let complete = if completed {
                bytes.len()
            } else {
                complete_prefix_len(&bytes)
            };
            // The style in force goes on from the last poll, but not past
            // a gap, whose bytes are not read.
            let mut pen = pen;
            let rendered = render_window(
                &output_window(&bytes[..complete], from_offset, output.start_offset, bound),
                |bytes, _, after_gap| {
                    if after_gap {
                        pen = TextStyle::default();
                    }
                    let rendered = render_styled_text(bytes, pen);
                    pen = rendered.pen;
                    (rendered.text, rendered.styles)
                },
            );
            let held = (bytes.len() - complete) as u64;
            (rendered, output.next_offset - held, None, pen)
        };
        let Rendered {
            text,
            styles,
            omitted,
        } = rendered;
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
            record.pen = pen;
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
            styles: (!styles.spans.is_empty()).then_some(styles),
        })
    }
}

/// A poll's text, its styles, and whether it has a gap.
struct Rendered {
    text: String,
    styles: TextStyles,
    omitted: bool,
}

/// Render each text part of a poll's `window` with `render` (given the part's
/// bytes, its offset, and whether a gap precedes it; returning its text and
/// styles), and each gap as its marker on a line of its own.
fn render_window(
    window: &[WindowPart<'_>],
    mut render: impl FnMut(&[u8], u64, bool) -> (String, TextStyles),
) -> Rendered {
    let mut text = String::new();
    let mut styles = TextStyles::default();
    let mut after_gap = false;
    for part in window {
        match part {
            WindowPart::Text { bytes, offset } => {
                let (part, part_styles) = render(bytes, *offset, after_gap);
                text.push_str(&part);
                styles.append(&part_styles);
                after_gap = false;
            }
            WindowPart::Omitted(omitted) => {
                let start = text.len();
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&omission_marker(*omitted));
                text.push('\n');
                styles.extend_plain(text.len() - start);
                after_gap = true;
            }
        }
    }
    let omitted = window
        .iter()
        .any(|part| matches!(part, WindowPart::Omitted(_)));
    Rendered {
        text,
        styles,
        omitted,
    }
}
