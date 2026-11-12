// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A form over several fields on one screen (Codex's provider editor,
//! `app/provider/editor.rs`, made generic): text fields, plain or secret,
//! choices, toggles and actions. Tab, Shift-Tab and the arrows move between
//! the fields; Enter moves on from a field and runs an action; Esc cancels.
//! The command whose form it is reads the values and sets the errors.

mod render;
mod text_field;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Line;

use crate::key_hint;
pub(crate) use text_field::TextField;

/// A field's name in its form.
pub(crate) type FieldId = &'static str;

/// What a key did to the form.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FormEvent {
    None,
    /// Enter on an action.
    Action(FieldId),
    Cancel,
}

pub(crate) struct FormItem {
    id: FieldId,
    label: String,
    kind: Kind,
    error: Option<String>,
}

enum Kind {
    Text {
        field: TextField,
        placeholder: String,
    },
    Choice {
        options: Vec<String>,
        selected: usize,
    },
    Toggle(bool),
    Action,
}

impl FormItem {
    pub(crate) fn text(id: FieldId, label: &str, value: &str) -> Self {
        Self::new(
            id,
            label,
            Kind::Text {
                field: TextField::new(value, false),
                placeholder: String::new(),
            },
        )
    }

    pub(crate) fn choice(id: FieldId, label: &str, options: &[&str], selected: usize) -> Self {
        Self::new(
            id,
            label,
            Kind::Choice {
                options: options.iter().map(|option| option.to_string()).collect(),
                selected: selected.min(options.len().saturating_sub(1)),
            },
        )
    }

    pub(crate) fn toggle(id: FieldId, label: &str, on: bool) -> Self {
        Self::new(id, label, Kind::Toggle(on))
    }

    pub(crate) fn action(id: FieldId, label: &str) -> Self {
        Self::new(id, label, Kind::Action)
    }

    /// A text field whose value is drawn as dots.
    pub(crate) fn secret(mut self) -> Self {
        if let Kind::Text { field, .. } = &mut self.kind {
            field.set_secret(true);
        }
        self
    }

    pub(crate) fn placeholder(mut self, text: &str) -> Self {
        if let Kind::Text { placeholder, .. } = &mut self.kind {
            *placeholder = text.to_string();
        }
        self
    }

    fn new(id: FieldId, label: &str, kind: Kind) -> Self {
        Self {
            id,
            label: label.to_string(),
            kind,
            error: None,
        }
    }
}

pub(crate) struct FormView {
    title: String,
    subtitle: Option<String>,
    items: Vec<FormItem>,
    /// The item with the focus.
    focus: usize,
    /// What the command says under the form: a test's result, an error.
    status: Vec<Line<'static>>,
}

impl FormView {
    pub(crate) fn new(title: impl Into<String>, items: Vec<FormItem>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            items,
            focus: 0,
            status: Vec::new(),
        }
    }

    pub(crate) fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> FormEvent {
        if key.kind == KeyEventKind::Release {
            return FormEvent::None;
        }
        let count = self.items.len().max(1);
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => return FormEvent::Cancel,
            (KeyCode::Tab | KeyCode::Down, KeyModifiers::NONE) => {
                self.focus = (self.focus + 1) % count;
                return FormEvent::None;
            }
            (KeyCode::BackTab, _)
            | (KeyCode::Tab, KeyModifiers::SHIFT)
            | (KeyCode::Up, KeyModifiers::NONE) => {
                self.focus = (self.focus + count - 1) % count;
                return FormEvent::None;
            }
            _ => {}
        }
        let enter = key_hint::plain(KeyCode::Enter).is_press(key);
        let space = key_hint::plain(KeyCode::Char(' ')).is_press(key);
        let mut advance = false;
        let id = self.items.get(self.focus).map(|item| item.id);
        match self.items.get_mut(self.focus).map(|item| &mut item.kind) {
            Some(Kind::Action) if enter => return id.map_or(FormEvent::None, FormEvent::Action),
            Some(Kind::Toggle(on)) if enter || space => *on = !*on,
            Some(Kind::Choice { options, selected }) => {
                let len = options.len().max(1);
                match key.code {
                    KeyCode::Right | KeyCode::Char(' ') => *selected = (*selected + 1) % len,
                    KeyCode::Left => *selected = (*selected + len - 1) % len,
                    KeyCode::Enter => advance = true,
                    _ => {}
                }
            }
            Some(Kind::Text { .. }) if enter => advance = true,
            Some(Kind::Text { field, .. }) => field.input(key),
            _ => {}
        }
        if advance {
            self.focus = (self.focus + 1) % count;
        }
        FormEvent::None
    }

    /// Pasted text goes to the focused text field, if one has the focus.
    pub(crate) fn handle_paste(&mut self, pasted: &str) -> bool {
        match self.items.get_mut(self.focus).map(|item| &mut item.kind) {
            Some(Kind::Text { field, .. }) => {
                field.paste(pasted);
                true
            }
            _ => false,
        }
    }

    /// The text of the field `id`, as typed.
    pub(crate) fn text(&self, id: FieldId) -> &str {
        match self.item(id).map(|item| &item.kind) {
            Some(Kind::Text { field, .. }) => field.text(),
            _ => "",
        }
    }

    pub(crate) fn choice(&self, id: FieldId) -> usize {
        match self.item(id).map(|item| &item.kind) {
            Some(Kind::Choice { selected, .. }) => *selected,
            _ => 0,
        }
    }

    pub(crate) fn toggle(&self, id: FieldId) -> bool {
        matches!(
            self.item(id).map(|item| &item.kind),
            Some(Kind::Toggle(true))
        )
    }

    /// Says what is wrong with `id` under it, and moves the focus there.
    pub(crate) fn set_error(&mut self, id: FieldId, error: impl Into<String>) {
        let Some(index) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        self.items[index].error = Some(error.into());
        self.focus = index;
    }

    pub(crate) fn clear_errors(&mut self) {
        for item in &mut self.items {
            item.error = None;
        }
    }

    pub(crate) fn set_status(&mut self, status: Vec<Line<'static>>) {
        self.status = status;
    }

    #[cfg(test)]
    pub(crate) fn error(&self, id: FieldId) -> Option<&str> {
        self.item(id).and_then(|item| item.error.as_deref())
    }

    fn item(&self, id: FieldId) -> Option<&FormItem> {
        self.items.iter().find(|item| item.id == id)
    }
}

#[cfg(test)]
mod tests;
