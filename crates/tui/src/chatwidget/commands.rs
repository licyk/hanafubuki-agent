// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The slash commands (Codex's `dispatch_command`), and what the chat does
//! with what they found: a session opened (and its transcript replayed),
//! the model and mode pickers, compaction and the reports.

use hanafubuki_agent::AgentSessionMode;

use super::ChatWidget;
use crate::agent_bridge::{ModelChoices, OpenedSession, ReplayedItem, TurnUpdate};
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;
use crate::slash_command::SlashCommand;
use crate::streaming::controller::{PlanStreamController, StreamController};

impl ChatWidget {
    pub(crate) fn dispatch_command(&mut self, cmd: SlashCommand) {
        if !cmd.available_during_task() && self.bottom_pane.is_task_running() {
            let message = format!(
                "'/{}' is disabled while a task is in progress.",
                cmd.command()
            );
            self.add_to_history(history_cell::new_error_event(message));
            self.request_redraw();
            return;
        }
        match cmd {
            SlashCommand::New => self.agent_command(AgentCommand::NewSession),
            SlashCommand::Resume => self.app_event_tx.send(AppEvent::OpenResumePicker),
            SlashCommand::Compact => self.agent_command(AgentCommand::Compact),
            SlashCommand::Model => self.agent_command(AgentCommand::ListModels),
            SlashCommand::Mode => self.open_mode_popup(),
            SlashCommand::Quit | SlashCommand::Exit => self.app_event_tx.send(AppEvent::Exit),
            SlashCommand::Diff => self.app_event_tx.send(AppEvent::Diff),
            SlashCommand::Mention => self.bottom_pane.insert_str("@"),
            SlashCommand::Skills => self.agent_command(AgentCommand::Skills),
            SlashCommand::Status => self.agent_command(AgentCommand::Status),
            SlashCommand::Mcp => self.agent_command(AgentCommand::Mcp),
        }
    }

    fn agent_command(&self, command: AgentCommand) {
        self.app_event_tx.send(AppEvent::Op(Op::Command(command)));
    }

    /// What the commands found; an update of the turn is handed back.
    pub(super) fn handle_command_update(&mut self, update: TurnUpdate) -> Option<TurnUpdate> {
        match update {
            TurnUpdate::SessionOpened(opened) => self.on_session_opened(opened),
            TurnUpdate::ModelChoices(choices) => self.open_model_popup(choices),
            TurnUpdate::ModelChanged(model) => self.add_to_history(history_cell::new_info_event(
                format!("Model changed to {model}"),
                None,
            )),
            TurnUpdate::ModeChanged(mode) => {
                self.mode = mode;
                self.add_to_history(history_cell::new_info_event(
                    format!("Mode set to {}", mode_name(mode)),
                    None,
                ));
            }
            TurnUpdate::Compacted(outcome) => match outcome.compaction.last_error {
                Some(error) => self.add_to_history(history_cell::new_error_event(format!(
                    "the conversation was not compacted: {error}"
                ))),
                None => self.add_to_history(history_cell::new_info_event(
                    "Context compacted".to_string(),
                    None,
                )),
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

    /// Shows the session from its start: the header, then what it said so
    /// far. What the previous session left running is gone with it.
    fn on_session_opened(&mut self, opened: OpenedSession) {
        self.running_tools.clear();
        self.stream_controller = None;
        self.plan_stream_controller = None;
        self.deferred.clear();
        self.bottom_pane.dismiss_all_requests();
        self.bottom_pane.set_task_running(false);
        self.retry_status_header = None;
        self.had_work_activity = false;
        self.mode = opened.mode;
        self.add_to_history(history_cell::SessionHeaderHistoryCell::new(
            opened.model,
            self.directory.clone(),
            env!("CARGO_PKG_VERSION"),
        ));
        if opened.earlier_items {
            self.add_to_history(history_cell::new_info_event(
                "Earlier messages are not shown.".to_string(),
                None,
            ));
        }
        let width = self.last_rendered_width.get();
        for item in opened.transcript {
            match item {
                ReplayedItem::User(text) => {
                    self.add_to_history(history_cell::new_user_prompt(text))
                }
                ReplayedItem::Assistant(text) => {
                    let mut controller = StreamController::new(width.map(|w| w.saturating_sub(2)));
                    controller.push(&text);
                    if let Some(cell) = controller.finalize() {
                        self.add_boxed_history(cell);
                    }
                }
                ReplayedItem::Plan(text) => {
                    let mut controller =
                        PlanStreamController::new(width.map(|w| w.saturating_sub(4)));
                    controller.push(&text);
                    if let Some(cell) = controller.finalize() {
                        self.add_boxed_history(cell);
                    }
                }
                ReplayedItem::Tool {
                    call_id,
                    name,
                    is_error,
                    result,
                } => {
                    let mut cell = history_cell::new_tool_call(call_id, name, None);
                    cell.complete(is_error, result);
                    self.add_to_history(cell);
                }
                ReplayedItem::Compacted => self.add_to_history(history_cell::new_info_event(
                    "Context compacted".to_string(),
                    None,
                )),
            }
        }
        self.needs_final_message_separator = false;
    }

    fn open_mode_popup(&mut self) {
        let items = [AgentSessionMode::Default, AgentSessionMode::Plan]
            .into_iter()
            .map(|mode| SelectionItem {
                name: mode_name(mode).to_string(),
                description: Some(
                    match mode {
                        AgentSessionMode::Default => "the Agent works on the task",
                        AgentSessionMode::Plan => {
                            "read-only exploration that ends with a proposed plan"
                        }
                    }
                    .to_string(),
                ),
                is_current: mode == self.mode,
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::Op(Op::Command(AgentCommand::SetMode(mode))))
                })],
                dismiss_on_select: true,
                ..Default::default()
            })
            .collect();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Select Mode".to_string()),
            subtitle: Some("Switch the mode of this session.".to_string()),
            items,
            ..Default::default()
        });
    }

    /// Codex's model picker over the Providers' catalogs: every model each
    /// lists, and the session's own when no catalog has it.
    fn open_model_popup(&mut self, choices: ModelChoices) {
        let mut items = Vec::new();
        let listed = choices.catalogs.iter().any(|catalog| {
            catalog.provider_id == choices.provider_id
                && catalog.models.iter().any(|entry| entry.id == choices.model)
        });
        if !listed && !choices.model.is_empty() {
            items.push(model_item(
                &choices.provider_id,
                &choices.model,
                choices.provider_id.clone(),
                true,
            ));
        }
        for catalog in &choices.catalogs {
            if let Some(error) = &catalog.error {
                items.push(SelectionItem {
                    name: catalog.display_name.clone(),
                    description: Some(format!("models not listed: {error}")),
                    is_disabled: true,
                    ..Default::default()
                });
                continue;
            }
            for entry in &catalog.models {
                let current =
                    catalog.provider_id == choices.provider_id && entry.id == choices.model;
                items.push(model_item(
                    &catalog.provider_id,
                    &entry.id,
                    catalog.display_name.clone(),
                    current,
                ));
            }
        }
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Select Model".to_string()),
            subtitle: Some("Switch the model of this session.".to_string()),
            items,
            is_searchable: true,
            search_placeholder: Some("Type to search models".to_string()),
            ..Default::default()
        });
    }
}

fn model_item(provider_id: &str, model: &str, provider: String, current: bool) -> SelectionItem {
    let provider_id = provider_id.to_string();
    let model = model.to_string();
    SelectionItem {
        name: model.clone(),
        description: Some(provider),
        is_current: current,
        search_value: Some(model.clone()),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::Op(Op::Command(AgentCommand::SetModel {
                provider_id: provider_id.clone(),
                model: model.clone(),
            })))
        })],
        dismiss_on_select: true,
        ..Default::default()
    }
}

fn mode_name(mode: AgentSessionMode) -> &'static str {
    match mode {
        AgentSessionMode::Default => "default",
        AgentSessionMode::Plan => "plan",
    }
}
