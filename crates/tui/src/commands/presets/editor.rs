// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A preset's form (Codex's quick-dial editor): its slot (for a new one),
//! its input and its label; Save checks the input as Codex does and saves
//! `tui.presets."N"`. And the presets the current settings make (Codex's
//! prefills): the session's permission mode, the terminal title's and the
//! status line's commands. The model's is left out (`/model` takes no
//! argument here).

use std::collections::BTreeMap;

use hanafubuki_agent::{AgentTuiPreferencesPatch, AgentTuiPreset};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;

use super::{free_slot, PRESETS, SLOTS};
use crate::app_event::{AppCommand, AppEvent, AppEventSender};
use crate::appearance::items::ids;
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, FormEvent, FormItem, FormView, SelectionItem,
    SelectionViewParams,
};
use crate::commands::{send_step, CommandContext};
use crate::history_cell::permission_mode_name;
use crate::render::renderable::Renderable;

const SLOT: &str = "slot";
const INPUT: &str = "input";
const LABEL: &str = "label";
const SAVE: &str = "save";

/// Opens the form for `slot`: `preset` to edit, or a new one.
pub(super) fn open_editor(
    context: &mut CommandContext<'_>,
    slot: char,
    preset: Option<AgentTuiPreset>,
    presets: BTreeMap<String, AgentTuiPreset>,
) {
    let editing = preset
        .as_ref()
        .filter(|_| presets.contains_key(&slot.to_string()))
        .is_some();
    let preset = preset.unwrap_or(AgentTuiPreset {
        input: String::new(),
        label: None,
    });
    let mut items = Vec::new();
    let title = if editing {
        format!("Edit preset {slot}")
    } else {
        let slots: Vec<String> = SLOTS.iter().map(char::to_string).collect();
        let options: Vec<&str> = slots.iter().map(String::as_str).collect();
        let selected = SLOTS.iter().position(|known| *known == slot).unwrap_or(0);
        items.push(FormItem::choice(SLOT, "Slot", &options, selected));
        "New preset".to_string()
    };
    items.push(
        FormItem::text(INPUT, "Input", &preset.input)
            .placeholder("a message, or a slash command such as /status"),
    );
    items.push(FormItem::text(
        LABEL,
        "Label",
        preset.label.as_deref().unwrap_or(""),
    ));
    items.push(FormItem::action(SAVE, "Save"));
    let form = FormView::new(title, items)
        .with_subtitle("A digit in /presets runs it as if it were typed and sent.");
    context.show_view(Box::new(PresetEditor {
        form,
        slot: editing.then_some(slot),
        presets,
        app_event_tx: context.app_event_tx(),
        complete: false,
    }));
}

/// The presets the current settings make, to pick one to start from.
pub(super) fn open_from_settings(
    context: &mut CommandContext<'_>,
    presets: BTreeMap<String, AgentTuiPreset>,
) {
    let appearance = context.appearance();
    let title = ids(appearance.terminal_title_items());
    let title = if title.is_empty() {
        Ok("/title --clear".to_string())
    } else {
        Ok(format!("/title {}", title.join(" ")))
    };
    let status = ids(appearance.status_line_items());
    let status = if status.is_empty() {
        Err("Choose status-line items with /statusline first.".to_string())
    } else {
        let colors = if appearance.status_line_colors() {
            "--colors"
        } else {
            "--no-colors"
        };
        Ok(format!("/statusline {} {colors}", status.join(" ")))
    };
    let permissions = appearance
        .values()
        .status
        .as_ref()
        .and_then(|status| status.permission_mode)
        .map(|mode| format!("/permissions {}", permission_mode_name(mode)))
        .ok_or_else(|| "No session yet: its permission mode is not known.".to_string());
    let model = Err("/model takes no argument in this interface.".to_string());
    let items = [
        ("Current permissions", permissions),
        ("Current terminal title", title),
        ("Current status line", status),
        ("Current model", model),
    ]
    .into_iter()
    .map(|(name, input)| match input {
        Ok(input) => {
            let presets = presets.clone();
            let shown = input.clone();
            SelectionItem {
                name: name.to_string(),
                description: Some(shown),
                actions: vec![Box::new(move |tx: &AppEventSender| {
                    let preset = AgentTuiPreset {
                        input: input.clone(),
                        label: None,
                    };
                    let presets = presets.clone();
                    send_step(tx, &PRESETS, move |context| {
                        let slot = free_slot(&presets);
                        open_editor(context, slot, Some(preset), presets)
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        }
        Err(reason) => SelectionItem {
            name: name.to_string(),
            is_disabled: true,
            disabled_reason: Some(reason),
            ..Default::default()
        },
    })
    .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Start a preset from the current settings".to_string()),
        items,
        ..Default::default()
    });
}

struct PresetEditor {
    form: FormView,
    /// The slot edited; `None` for a new preset, whose slot is the form's.
    slot: Option<char>,
    presets: BTreeMap<String, AgentTuiPreset>,
    app_event_tx: AppEventSender,
    complete: bool,
}

impl PresetEditor {
    fn save(&mut self) {
        self.form.clear_errors();
        let slot = self
            .slot
            .unwrap_or_else(|| SLOTS[self.form.choice(SLOT).min(SLOTS.len() - 1)]);
        if self.slot.is_none() && self.presets.contains_key(&slot.to_string()) {
            self.form.set_error(
                SLOT,
                format!("Slot {slot} holds a preset: choose another, or edit or delete that one."),
            );
            return;
        }
        let label = self.form.text(LABEL).trim();
        let preset = AgentTuiPreset {
            input: self.form.text(INPUT).to_string(),
            label: (!label.is_empty()).then(|| label.to_string()),
        };
        if let Err(error) = preset.validate() {
            self.form.set_error(INPUT, capitalized(&error));
            return;
        }
        let patch = AgentTuiPreferencesPatch {
            presets: BTreeMap::from([(slot.to_string(), Some(preset))]),
            ..AgentTuiPreferencesPatch::default()
        };
        let message = format!("Saved preset {slot}.");
        self.app_event_tx
            .send(AppEvent::Command(AppCommand::SavePreferences(
                patch, message,
            )));
        self.complete = true;
    }
}

impl BottomPaneView for PresetEditor {
    fn handle_key_event(&mut self, key: KeyEvent) {
        match self.form.handle_key(key) {
            FormEvent::Cancel => self.complete = true,
            FormEvent::Action(_) => self.save(),
            FormEvent::None => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        self.form.handle_paste(&pasted)
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for PresetEditor {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.form.render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.form.desired_height(width)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.form.cursor_pos(area)
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
