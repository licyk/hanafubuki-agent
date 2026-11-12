// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/memories` (Codex's `bottom_pane/memories_settings_view.rs`): the
//! settings center's Memory tab, whose settings (`memory.enabled`,
//! `memory.useMemories`, `memory.generateMemories`, …) are rows like any
//! other. Its Memory store page is the rest: the memory's status, whether
//! this session contributes, and Reset after a confirmation.

use hanafubuki_agent::{AgentMemoryContributionMode, AgentMemoryStatus, SettingsGroup};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{MemoriesState, SettingsCall, SettingsReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{GenericDisplayRow, MenuKey, MenuModel, MenuView};
use crate::settings_center::{Attach, PageOverride};

pub(crate) static MEMORIES: CommandSpec = CommandSpec {
    name: "memories",
    aliases: &[],
    description: "use, generate or reset the memories",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| super::open_settings(context, "memory"),
};

/// The settings center's page of the memory store.
pub(crate) static PAGE: PageOverride = PageOverride {
    id: "memory-store",
    attach: Attach::Row {
        group: SettingsGroup::Memory,
        label: "Memory store",
        summary: "Its status, this session's contribution, and a reset",
    },
    command: &MEMORIES,
    open: |context, _| {
        let view = MenuView::new(MemoriesPanel::default(), context.app_event_tx());
        context.show_view(Box::new(view));
        context.agent_command(AgentCommand::Settings(SettingsCall::Memories));
    },
};

/// The state read, into the open page; with none open, it is dropped.
pub(super) fn on_reply(context: &mut CommandContext<'_>, state: MemoriesState) {
    let _ = context.offer_reply(CommandReply::Settings(SettingsReply::Memories(Box::new(
        state,
    ))));
}

#[derive(Default)]
pub(super) struct MemoriesPanel {
    /// `None` until it is read.
    state: Option<MemoriesState>,
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
    Contribution,
    Reset,
    KeepMemories,
    ConfirmReset,
}

impl MemoriesPanel {
    fn row_kinds(&self) -> Vec<Row> {
        if self.state.is_none() {
            Vec::new()
        } else if self.confirming_reset {
            vec![Row::KeepMemories, Row::ConfirmReset]
        } else {
            vec![Row::Contribution, Row::Reset]
        }
    }

    fn activate(&mut self, row: Row, tx: &AppEventSender) {
        let Some(state) = &self.state else {
            return;
        };
        let call = match row {
            Row::Contribution => match state.contribution {
                Some((mode, revision)) if mode != AgentMemoryContributionMode::Polluted => {
                    SettingsCall::SetContribution {
                        enabled: !mode.contributes(),
                        revision,
                    }
                }
                _ => return,
            },
            Row::Reset => {
                self.confirming_reset = state.status.is_ok();
                return;
            }
            Row::KeepMemories => {
                self.confirming_reset = false;
                return;
            }
            Row::ConfirmReset => {
                self.confirming_reset = false;
                let Ok(status) = &state.status else {
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
            "Memory store".to_string()
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
        let Some(state) = &self.state else {
            lines.push(Line::from("Reading the memory…".dim()));
            return lines;
        };
        if !state.enabled {
            lines.push(Line::from(
                "Memory is off: turn on memory.enabled in the Memory tab.".dim(),
            ));
        }
        match &state.status {
            Ok(status) => lines.push(Line::from(status_line(status).dim())),
            Err(error) => lines.push(Line::from(format!("Status: {error}").red())),
        }
        if let Ok(AgentMemoryStatus {
            last_error: Some(error),
            ..
        }) = &state.status
        {
            lines.push(Line::from(format!("Latest error: {error}").red()));
        }
        lines.extend(state.note.clone().map(|note| Line::from(note.green())));
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
        let contribution = self.state.as_ref().and_then(|state| state.contribution);
        self.row_kinds()
            .into_iter()
            .map(|row| match row {
                Row::Contribution => match contribution {
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

    fn empty_message(&self) -> &'static str {
        "…"
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
                self.state = Some(*state);
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
