// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/presets` (Codex's quick dials, `chatwidget/quick_dials.rs` and
//! `app/quick_dials.rs`): inputs kept in slots 0 to 9 (`tui.presets`),
//! each run as if it were typed and sent, a message or a slash command.
//! The menu lists the slots; a digit runs its slot, `n` writes a new one,
//! `s` starts one from the current settings, `e` edits the selected one and
//! `d` deletes it after a confirmation. The keymap's `chat.open_presets`
//! (Ctrl-X P) opens the menu.

mod editor;

use std::collections::BTreeMap;

use hanafubuki_agent::{AgentTuiPreferencesPatch, AgentTuiPreset};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{send_step, CommandContext, CommandSpec};
use crate::app_event::{AppCommand, AppEvent, AppEventSender};
use crate::bottom_pane::{GenericDisplayRow, MenuKey, MenuModel, MenuView};
use crate::history_cell;

pub(crate) static PRESETS: CommandSpec = CommandSpec {
    name: "presets",
    aliases: &[],
    description: "run, save or edit the inputs kept in slots 0 to 9",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

/// The slots in the menu's order: 1 to 9, then 0, as the keyboard has them.
const SLOTS: [char; 10] = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'];

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    let menu = PresetsMenu {
        presets: context.preferences().presets.clone(),
        deleting: None,
    };
    let view = MenuView::new(menu, context.app_event_tx());
    context.show_view(Box::new(view));
}

pub(super) struct PresetsMenu {
    presets: BTreeMap<String, AgentTuiPreset>,
    /// `d` was pressed on this slot: the menu asks first.
    deleting: Option<char>,
}

/// What the menu shows of a preset: its label, or its input's first line.
pub(super) fn preset_name(preset: &AgentTuiPreset) -> String {
    preset
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or(&preset.input)
        .trim_start()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The first slot without a preset, else the first.
pub(super) fn free_slot(presets: &BTreeMap<String, AgentTuiPreset>) -> char {
    SLOTS
        .into_iter()
        .find(|slot| !presets.contains_key(&slot.to_string()))
        .unwrap_or(SLOTS[0])
}

impl PresetsMenu {
    fn preset(&self, slot: char) -> Option<&AgentTuiPreset> {
        self.presets.get(&slot.to_string())
    }

    /// Runs the preset in `slot`, checked as a saved one is.
    fn run(&self, slot: char, tx: &AppEventSender) -> MenuKey {
        let Some(preset) = self.preset(slot) else {
            return MenuKey::Handled;
        };
        match preset.validate() {
            Ok(()) => {
                tx.send(AppEvent::Command(AppCommand::Replay(preset.input.clone())));
                MenuKey::Close
            }
            Err(error) => {
                let cell = history_cell::new_error_event(format!("Preset {slot}: {error}"));
                tx.send(AppEvent::InsertHistoryCell(Box::new(cell)));
                MenuKey::Handled
            }
        }
    }

    fn delete_keys(&mut self, slot: char, key: KeyEvent, tx: &AppEventSender) -> MenuKey {
        self.deleting = None;
        if matches!(key.code, KeyCode::Char('y') | KeyCode::Enter) {
            let patch = AgentTuiPreferencesPatch {
                presets: BTreeMap::from([(slot.to_string(), None)]),
                ..AgentTuiPreferencesPatch::default()
            };
            let message = format!("Deleted preset {slot}.");
            tx.send(AppEvent::Command(AppCommand::SavePreferences(
                patch, message,
            )));
            self.presets.remove(&slot.to_string());
        }
        MenuKey::Handled
    }
}

impl MenuModel for PresetsMenu {
    fn title(&self) -> String {
        "Presets".to_string()
    }

    fn header(&self) -> Vec<Line<'static>> {
        match self.deleting {
            Some(slot) => vec![Line::from(
                format!("Delete preset {slot}? y deletes it, any other key keeps it.").red(),
            )],
            None if self.presets.is_empty() => {
                vec![Line::from("No presets saved. Press n to create one.".dim())]
            }
            None => Vec::new(),
        }
    }

    fn rows(&self) -> Vec<GenericDisplayRow> {
        SLOTS
            .into_iter()
            .map(|slot| match self.preset(slot) {
                Some(preset) => GenericDisplayRow {
                    name: format!("{slot}  {}", preset_name(preset)),
                    description: preset
                        .label
                        .as_ref()
                        .map(|_| preset.input.lines().next().unwrap_or_default().to_string()),
                    ..Default::default()
                },
                None => GenericDisplayRow {
                    name: format!("{slot}  empty"),
                    is_disabled: true,
                    ..Default::default()
                },
            })
            .collect()
    }

    fn hints(&self) -> String {
        "0-9 or enter run · n new · s from settings · e edit · d delete · esc close".to_string()
    }

    fn on_key(&mut self, key: KeyEvent, selected: Option<usize>, tx: &AppEventSender) -> MenuKey {
        if let Some(slot) = self.deleting {
            return self.delete_keys(slot, key, tx);
        }
        if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
            return MenuKey::Ignored;
        }
        let selected = selected.and_then(|index| SLOTS.get(index).copied());
        let edit = |slot: char, preset: Option<AgentTuiPreset>, presets: &BTreeMap<_, _>| {
            let presets = presets.clone();
            send_step(tx, &PRESETS, move |context| {
                editor::open_editor(context, slot, preset, presets)
            });
            MenuKey::Close
        };
        match key.code {
            KeyCode::Char(digit) if digit.is_ascii_digit() => self.run(digit, tx),
            KeyCode::Enter => selected.map_or(MenuKey::Ignored, |slot| self.run(slot, tx)),
            KeyCode::Char('n') => {
                let slot = selected
                    .filter(|slot| self.preset(*slot).is_none())
                    .unwrap_or_else(|| free_slot(&self.presets));
                edit(slot, None, &self.presets)
            }
            KeyCode::Char('e') => {
                match selected.and_then(|slot| Some((slot, self.preset(slot)?))) {
                    Some((slot, preset)) => edit(slot, Some(preset.clone()), &self.presets),
                    None => MenuKey::Handled,
                }
            }
            KeyCode::Char('d') => {
                self.deleting = selected.filter(|slot| self.preset(*slot).is_some());
                MenuKey::Handled
            }
            KeyCode::Char('s') => {
                let presets = self.presets.clone();
                send_step(tx, &PRESETS, move |context| {
                    editor::open_from_settings(context, presets)
                });
                MenuKey::Close
            }
            _ => MenuKey::Ignored,
        }
    }
}
