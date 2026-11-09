// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The slash commands' dispatch (Codex's `dispatch_command`), each
//! command run as an isolated module, and what the chat does with what
//! they found: a session opened (and its transcript replayed), the model
//! picker, compaction and the reports.

use hanafubuki_agent::panic::catch_panic;
use hanafubuki_agent::AgentCompactionStatus;

use super::ChatWidget;
use crate::agent_bridge::TurnUpdate;
use crate::app_event::{AppEvent, Op};
use crate::commands::{self, CommandContext, CommandSpec};
use crate::history_cell;

impl ChatWidget {
    pub(crate) fn dispatch_command(&mut self, command: &'static CommandSpec, args: String) {
        if !command.during_task && self.bottom_pane.is_task_running() {
            let message = format!(
                "'/{}' is disabled while a task is in progress.",
                command.name
            );
            self.add_to_history(history_cell::new_error_event(message));
            self.request_redraw();
            return;
        }
        if command.needs_session && !self.bottom_pane.command_disabled(command.name) {
            self.app_event_tx.send(AppEvent::Op(Op::EnsureSession));
        }
        self.run_command(command, |context| (command.dispatch)(context, &args));
    }

    /// Runs `call` as `command`'s: a panic in it turns the command off
    /// instead of ending the interface. A command turned off runs nothing.
    fn run_command(
        &mut self,
        command: &'static CommandSpec,
        call: impl FnOnce(&mut CommandContext<'_>),
    ) {
        if self.bottom_pane.command_disabled(command.name) {
            return;
        }
        let mut context = CommandContext::new(
            command,
            &self.app_event_tx,
            &mut self.bottom_pane,
            self.mode,
            self.fullscreen_preference,
        );
        if let Err(message) = catch_panic(|| call(&mut context)) {
            self.command_failed(command.name, &message);
        }
        self.request_redraw();
    }

    /// The views of commands that panicked since the last look (drawing
    /// them, or handling a key): those commands are turned off.
    pub(crate) fn reap_failed_views(&mut self) {
        for failure in self.bottom_pane.take_view_failures() {
            self.command_failed(failure.command, &failure.message);
        }
    }

    /// `/name` panicked: it is turned off for the rest of the run, its
    /// views closed, and the transcript and the warnings say so.
    fn command_failed(&mut self, name: &'static str, message: &str) {
        self.bottom_pane.disable_command(name);
        let warning =
            format!("'/{name}' failed and is turned off for the rest of this run: {message}");
        self.add_to_history(history_cell::new_error_event(warning.clone()));
        self.warnings.push(warning);
        self.request_redraw();
    }

    /// What the commands found; an update of the turn is handed back.
    pub(super) fn handle_command_update(&mut self, update: TurnUpdate) -> Option<TurnUpdate> {
        match update {
            TurnUpdate::SessionOpened(opened) => self.on_session_opened(opened),
            TurnUpdate::ModelChoices(choices) => self.run_command(&commands::MODEL, |context| {
                commands::show_model_choices(context, choices)
            }),
            TurnUpdate::ModelChanged(model) => self.add_to_history(history_cell::new_info_event(
                format!("Model changed to {model}"),
                None,
            )),
            TurnUpdate::ModeChanged(mode) => {
                self.mode = mode;
                self.add_to_history(history_cell::new_info_event(
                    format!("Mode set to {}", commands::mode_name(mode)),
                    None,
                ));
            }
            TurnUpdate::Compacted(outcome) => match outcome.compaction.status {
                AgentCompactionStatus::Completed => self.add_to_history(
                    history_cell::new_info_event("Context compacted".to_string(), None),
                ),
                status => {
                    let reason = outcome
                        .compaction
                        .last_error
                        .unwrap_or_else(|| format!("{status:?}").to_lowercase());
                    self.add_to_history(history_cell::new_error_event(format!(
                        "the conversation was not compacted: {reason}"
                    )))
                }
            },
            TurnUpdate::StatusReport(report) => {
                self.add_to_history(history_cell::new_status_output(&report))
            }
            TurnUpdate::Mcp(snapshot) => {
                self.add_to_history(history_cell::new_mcp_output(&snapshot))
            }
            TurnUpdate::Skills(catalog) => {
                self.add_to_history(history_cell::new_skills_output(&catalog))
            }
            update => return Some(update),
        }
        None
    }
}
