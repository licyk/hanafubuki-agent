// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/warnings` (Codex's `bottom_pane/warnings_view.rs`): the warnings kept
//! in this run, newest first, each once, counted by source: the
//! configuration (its recoveries, what its layers could not use), the
//! plugins, MCP, the hooks, the panics, and the interface's own. `c`
//! copies the one selected, `d` dismisses it for the rest of the run.

use std::collections::BTreeMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::{SettingsCall, Warning, WarningSource};
use crate::app_event::{AgentCommand, AppCommand, AppEvent, AppEventSender};
use crate::bottom_pane::{GenericDisplayRow, MenuKey, MenuModel, MenuView};
use crate::history_cell;

pub(crate) static WARNINGS: CommandSpec = CommandSpec {
    name: "warnings",
    aliases: &[],
    description: "show the warnings of this run",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    context.agent_command(AgentCommand::Settings(SettingsCall::Warnings));
}

/// Shows `warnings` (already merged, deduplicated and sorted), or says
/// there are none.
pub(crate) fn show_warnings(context: &mut CommandContext<'_>, warnings: Vec<Warning>) {
    if warnings.is_empty() {
        context.add_to_history(history_cell::new_info_event(
            "No warnings.".to_string(),
            None,
        ));
        return;
    }
    let view = MenuView::new(WarningsList { warnings }, context.app_event_tx());
    context.show_view(Box::new(view));
}

pub(super) struct WarningsList {
    warnings: Vec<Warning>,
}

impl MenuModel for WarningsList {
    fn title(&self) -> String {
        let count = self.warnings.len();
        format!("{count} warning{}", if count == 1 { "" } else { "s" })
    }

    fn header(&self) -> Vec<Line<'static>> {
        let mut counts: BTreeMap<WarningSource, usize> = BTreeMap::new();
        for warning in &self.warnings {
            *counts.entry(warning.source).or_default() += 1;
        }
        let counts: Vec<String> = counts
            .into_iter()
            .map(|(source, count)| format!("{count} {}", source.name()))
            .collect();
        vec![Line::from(counts.join(" · ").dim())]
    }

    fn rows(&self) -> Vec<GenericDisplayRow> {
        self.warnings
            .iter()
            .map(|warning| GenericDisplayRow {
                name: format!("[{}] {}", warning.source.name(), warning.text),
                wrap_indent: Some(4),
                ..Default::default()
            })
            .collect()
    }

    fn hints(&self) -> String {
        "c copy · d dismiss for this run · esc close".to_string()
    }

    fn on_key(&mut self, key: KeyEvent, selected: Option<usize>, tx: &AppEventSender) -> MenuKey {
        let Some(index) = selected.filter(|index| *index < self.warnings.len()) else {
            return MenuKey::Ignored;
        };
        match key.code {
            KeyCode::Char('c') | KeyCode::Enter => {
                let text = self.warnings[index].text.clone();
                tx.send(AppEvent::Command(AppCommand::Copy(text)));
                MenuKey::Handled
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let warning = self.warnings.remove(index);
                tx.send(AppEvent::Command(AppCommand::DismissWarning(warning.key())));
                if self.warnings.is_empty() {
                    MenuKey::Close
                } else {
                    MenuKey::Handled
                }
            }
            _ => MenuKey::Ignored,
        }
    }

    fn empty_message(&self) -> &'static str {
        "No warnings."
    }
}
