// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A command's process can keep running after its tool call has returned.

use hanafubuki_agent::{AgentExecutionProgressStatus, AgentToolDisplay};
use ratatui::style::Stylize;
use ratatui::text::Line;
use std::time::Duration;

use super::{command, text, Extent, LiveOutput, ToolCall, ToolCallCell, View};
use crate::history_cell::ActivityDisclosure;
use crate::status_indicator_widget::fmt_elapsed_tenths;

#[derive(Debug)]
pub(super) struct Execution {
    status: AgentExecutionProgressStatus,
    exit_code: Option<i32>,
    elapsed: Duration,
}

impl Execution {
    pub(super) fn interrupt(&mut self, elapsed: Duration) {
        if !self.status.is_terminal() {
            self.status = AgentExecutionProgressStatus::Cancelled;
            self.elapsed = elapsed;
        }
    }
}

impl ToolCallCell {
    pub(crate) fn execution_status(
        &mut self,
        call_id: &str,
        status: AgentExecutionProgressStatus,
        exit_code: Option<i32>,
    ) {
        if let Some(call) = self.call_mut(call_id) {
            call.execution = Some(Execution {
                status,
                exit_code,
                elapsed: call.start_time.elapsed(),
            });
        }
    }
}

impl ToolCall {
    pub(super) fn is_running(&self) -> bool {
        self.execution
            .as_ref()
            .map_or(self.outcome.is_none(), |execution| {
                !execution.status.is_terminal()
            })
    }

    pub(super) fn execution_view(&self, extent: Extent) -> Option<View> {
        let execution = self.execution.as_ref()?;
        // A foreground command's final result has the complete, styled output.
        // A yielded result is only a slice; keep the monitor's stream instead.
        if self
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.display.as_ref())
            .is_some_and(|display| {
                matches!(
                    display,
                    AgentToolDisplay::Command {
                        exit_code: Some(_),
                        ..
                    }
                )
            })
        {
            return None;
        }
        let failed = execution.status.is_terminal()
            && (execution.status != AgentExecutionProgressStatus::Exited
                || execution.exit_code.is_some_and(|code| code != 0));
        let header = if self.is_running() {
            text::running(self)
        } else {
            text::called(self, failed)
        };
        let empty = LiveOutput::default();
        let mut view =
            command::streamed(self, header, self.live.as_ref().unwrap_or(&empty), extent);
        if view.preview.is_empty() {
            view.preview.push(if self.is_running() {
                self.placeholder()
            } else {
                Line::from("(no output)".dim())
            });
        }
        if execution.status.is_terminal() {
            let status = match execution.exit_code {
                Some(0) => "✓".to_string(),
                Some(code) => format!("exit {code}"),
                None => match execution.status {
                    AgentExecutionProgressStatus::TimedOut => "timed out",
                    AgentExecutionProgressStatus::Cancelled => "cancelled",
                    AgentExecutionProgressStatus::Failed => "failed",
                    _ => "exited",
                }
                .to_string(),
            };
            let ending = format!("{status} · {}", fmt_elapsed_tenths(execution.elapsed));
            let ending = Line::from(if failed { ending.red() } else { ending.dim() });
            if failed {
                view.preview.push(ending.clone());
            }
            view.details.push(ending);
        }
        view.hidden = Some(ActivityDisclosure::Generic);
        Some(view)
    }
}
