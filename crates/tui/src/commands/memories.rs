// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/memories` (Codex's `bottom_pane/memories_settings_view.rs`): whether
//! the model uses the memories and whether sessions are read for new ones
//! (`memory.useMemories`, `memory.generateMemories`), whether this session
//! contributes, the memory's status, and Reset after a confirmation. With
//! memory off (`memory.enabled`), the panel offers to turn it on.

use hanafubuki_agent::{AgentMemoryContributionMode, AgentMemoryStatus};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{MemoriesState, SettingsCall, SettingsReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{GenericDisplayRow, MenuKey, MenuModel, MenuView};

pub(crate) static MEMORIES: CommandSpec = CommandSpec {
    name: "memories",
    aliases: &[],
    description: "use, generate or reset the memories",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    context.agent_command(AgentCommand::Settings(SettingsCall::Memories));
}

/// The state read: into the open panel, or a new one.
pub(super) fn on_reply(context: &mut CommandContext<'_>, state: MemoriesState) {
    let reply = CommandReply::Settings(SettingsReply::Memories(Box::new(state)));
    if let Some(CommandReply::Settings(SettingsReply::Memories(state))) = context.offer_reply(reply)
    {
        let view = MenuView::new(MemoriesPanel::new(*state), context.app_event_tx());
        context.show_view(Box::new(view));
    }
}

pub(super) struct MemoriesPanel {
    state: MemoriesState,
    /// Reset was chosen: the panel asks first.
    confirming_reset: bool,
    /// A change was sent: the panel waits for what was saved, so a change
    /// is always made to the state as saved.
    saving: bool,
    /// Why the last change failed.
    error: Option<String>,
}

/// What a row does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    TurnOn,
    Use,
    Generate,
    Contribution,
    Reset,
    KeepMemories,
    ConfirmReset,
}

impl MemoriesPanel {
    fn new(state: MemoriesState) -> Self {
        Self {
            state,
            confirming_reset: false,
            saving: false,
            error: None,
        }
    }

    fn row_kinds(&self) -> Vec<Row> {
        if self.confirming_reset {
            vec![Row::KeepMemories, Row::ConfirmReset]
        } else if !self.state.settings.enabled {
            vec![Row::TurnOn]
        } else {
            vec![Row::Use, Row::Generate, Row::Contribution, Row::Reset]
        }
    }

    fn activate(&mut self, row: Row, tx: &AppEventSender) {
        let mut settings = self.state.settings.clone();
        let call = match row {
            Row::TurnOn => {
                settings.enabled = true;
                SettingsCall::SetMemory(settings)
            }
            Row::Use => {
                settings.use_memories = !settings.use_memories;
                SettingsCall::SetMemory(settings)
            }
            Row::Generate => {
                settings.generate_memories = !settings.generate_memories;
                SettingsCall::SetMemory(settings)
            }
            Row::Contribution => match self.state.contribution {
                Some((mode, revision)) if mode != AgentMemoryContributionMode::Polluted => {
                    SettingsCall::SetContribution {
                        enabled: !mode.contributes(),
                        revision,
                    }
                }
                _ => return,
            },
            Row::Reset => {
                self.confirming_reset = self.state.status.is_ok();
                return;
            }
            Row::KeepMemories => {
                self.confirming_reset = false;
                return;
            }
            Row::ConfirmReset => {
                self.confirming_reset = false;
                let Ok(status) = &self.state.status else {
                    return;
                };
                SettingsCall::ResetMemory {
                    generation: status.generation,
                }
            }
        };
        self.saving = true;
        self.error = None;
        tx.send(AppEvent::Op(Op::Command(AgentCommand::Settings(call))));
    }
}

impl MenuModel for MemoriesPanel {
    fn title(&self) -> String {
        if self.confirming_reset {
            "Reset memory?".to_string()
        } else {
            "Memories".to_string()
        }
    }

    fn header(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if self.confirming_reset {
            lines.push(Line::from(
                "The memories and their extractions are deleted, and every session is read again."
                    .dim(),
            ));
            return lines;
        }
        if !self.state.settings.enabled {
            lines.push(Line::from("Memory is off (memory.enabled).".dim()));
        }
        match &self.state.status {
            Ok(status) => lines.push(Line::from(status_line(status).dim())),
            Err(error) => lines.push(Line::from(format!("Status: {error}").red())),
        }
        if let Ok(AgentMemoryStatus {
            last_error: Some(error),
            ..
        }) = &self.state.status
        {
            lines.push(Line::from(format!("Latest error: {error}").red()));
        }
        lines.extend(self.state.note.clone().map(|note| Line::from(note.green())));
        lines.extend(self.error.clone().map(|error| Line::from(error.red())));
        if self.saving {
            lines.push(Line::from("Saving…".dim()));
        }
        lines
    }

    fn rows(&self) -> Vec<GenericDisplayRow> {
        let toggle = |on: bool, name: &str, description: String| GenericDisplayRow {
            name: format!("[{}] {name}", if on { 'x' } else { ' ' }),
            description: Some(description),
            ..Default::default()
        };
        let settings = &self.state.settings;
        self.row_kinds()
            .into_iter()
            .map(|row| match row {
                Row::TurnOn => GenericDisplayRow {
                    name: "Turn memory on".to_string(),
                    description: Some("memory.enabled = true".to_string()),
                    ..Default::default()
                },
                Row::Use => toggle(
                    settings.use_memories,
                    "Use memories",
                    "the model sees the memories (memory.useMemories)".to_string(),
                ),
                Row::Generate => toggle(
                    settings.generate_memories,
                    "Generate memories",
                    "sessions are read for new memories (memory.generateMemories)".to_string(),
                ),
                Row::Contribution => match self.state.contribution {
                    Some((AgentMemoryContributionMode::Polluted, _)) => GenericDisplayRow {
                        name: "[ ] This session contributes".to_string(),
                        is_disabled: true,
                        disabled_reason: Some(
                            "it used outside content, so it never contributes".to_string(),
                        ),
                        ..Default::default()
                    },
                    Some((mode, _)) => toggle(
                        mode.contributes(),
                        "This session contributes",
                        "whether this session is read for memories".to_string(),
                    ),
                    None => GenericDisplayRow {
                        name: "[ ] This session contributes".to_string(),
                        is_disabled: true,
                        disabled_reason: Some("no session yet".to_string()),
                        ..Default::default()
                    },
                },
                Row::Reset => GenericDisplayRow {
                    name: "Reset memory…".to_string(),
                    description: Some("delete the memories, after a confirmation".to_string()),
                    ..Default::default()
                },
                Row::KeepMemories => GenericDisplayRow {
                    name: "No, keep them".to_string(),
                    ..Default::default()
                },
                Row::ConfirmReset => GenericDisplayRow {
                    name: "Yes, reset memory".to_string(),
                    ..Default::default()
                },
            })
            .collect()
    }

    fn hints(&self) -> String {
        "space or enter change · esc close".to_string()
    }

    fn on_key(&mut self, key: KeyEvent, selected: Option<usize>, tx: &AppEventSender) -> MenuKey {
        if self.saving || !matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
            return MenuKey::Ignored;
        }
        let Some(row) = selected.and_then(|index| self.row_kinds().get(index).copied()) else {
            return MenuKey::Ignored;
        };
        self.activate(row, tx);
        MenuKey::Handled
    }

    fn on_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match reply {
            CommandReply::Settings(SettingsReply::Memories(state)) => {
                self.state = *state;
                self.saving = false;
                None
            }
            CommandReply::Settings(SettingsReply::Failed { error, .. }) => {
                self.error = Some(error);
                self.saving = false;
                None
            }
            reply => Some(reply),
        }
    }
}

/// `12 sessions read · 9 memories · last pass 2026-11-09 14:03`.
fn status_line(status: &AgentMemoryStatus) -> String {
    let mut parts = vec![
        format!("{} sessions read", status.source_count),
        format!("{} memories", status.phase_one_count),
    ];
    if status.running_jobs > 0 {
        parts.push(format!("{} running", status.running_jobs));
    }
    let last = status
        .last_success_at_millis
        .and_then(|millis| chrono::DateTime::from_timestamp_millis(millis as i64))
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        });
    parts.push(match last {
        Some(at) => format!("last pass {at}"),
        None => "no pass yet".to_string(),
    });
    parts.join(" · ")
}
