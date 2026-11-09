// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A form over several fields on one screen (Codex's provider editor,
//! `app/provider/editor.rs`, made generic): text fields, plain or secret,
//! choices, toggles and lists of name and value rows, an "Advanced" part
//! drawn collapsed, and actions. Tab, Shift-Tab and the arrows move between
//! the stops; Enter moves on from a field and runs an action; Esc cancels.
//! The command whose form it is reads the values and sets the errors.

mod render;
mod text_field;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Line;

use crate::key_hint;
use text_field::TextField;

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
    advanced: bool,
    /// Shown only while the choice `.0` has its option `.1` selected.
    shown_if: Option<(FieldId, usize)>,
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
    Rows {
        rows: Vec<Row>,
        secret_column: bool,
        add_label: String,
    },
    Action,
}

struct Row {
    name: TextField,
    value: TextField,
}

/// A row as read from the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RowValue {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) secret: bool,
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

    /// Rows of a name and a value; `add_label` names the stop that adds one.
    pub(crate) fn rows(id: FieldId, label: &str, rows: Vec<RowValue>, add_label: &str) -> Self {
        let rows = rows
            .into_iter()
            .map(|row| Row {
                name: TextField::new(&row.name, false),
                value: TextField::new(&row.value, row.secret),
            })
            .collect();
        Self::new(
            id,
            label,
            Kind::Rows {
                rows,
                secret_column: false,
                add_label: add_label.to_string(),
            },
        )
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

    /// Rows each marked public or secret, a secret's value drawn as dots.
    pub(crate) fn with_secret_column(mut self) -> Self {
        if let Kind::Rows { secret_column, .. } = &mut self.kind {
            *secret_column = true;
        }
        self
    }

    pub(crate) fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }

    pub(crate) fn shown_if(mut self, choice: FieldId, option: usize) -> Self {
        self.shown_if = Some((choice, option));
        self
    }

    fn new(id: FieldId, label: &str, kind: Kind) -> Self {
        Self {
            id,
            label: label.to_string(),
            kind,
            advanced: false,
            shown_if: None,
            error: None,
        }
    }
}

/// Where the focus can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Item(usize),
    RowName(usize, usize),
    RowValue(usize, usize),
    RowSecret(usize, usize),
    AddRow(usize),
    Advanced,
}

pub(crate) struct FormView {
    title: String,
    subtitle: Option<String>,
    items: Vec<FormItem>,
    focus: Stop,
    advanced_open: bool,
    /// What the command says under the form: a test's result, an error.
    status: Vec<Line<'static>>,
}

impl FormView {
    pub(crate) fn new(title: impl Into<String>, items: Vec<FormItem>) -> Self {
        let mut form = Self {
            title: title.into(),
            subtitle: None,
            items,
            focus: Stop::Item(0),
            advanced_open: false,
            status: Vec::new(),
        };
        form.focus = form.stops().first().copied().unwrap_or(Stop::Item(0));
        form
    }

    pub(crate) fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> FormEvent {
        if key.kind == KeyEventKind::Release {
            return FormEvent::None;
        }
        let stops = self.stops();
        let position = stops.iter().position(|stop| *stop == self.focus);
        let count = stops.len().max(1);
        let step = |by: usize| position.map_or(0, |position| (position + by) % count);
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => return FormEvent::Cancel,
            (KeyCode::Tab | KeyCode::Down, KeyModifiers::NONE) => {
                self.focus_stop(&stops, step(1));
                return FormEvent::None;
            }
            (KeyCode::BackTab, _)
            | (KeyCode::Tab, KeyModifiers::SHIFT)
            | (KeyCode::Up, KeyModifiers::NONE) => {
                self.focus_stop(&stops, step(count - 1));
                return FormEvent::None;
            }
            _ => {}
        }
        let enter = key_hint::plain(KeyCode::Enter).is_press(key);
        let space = key_hint::plain(KeyCode::Char(' ')).is_press(key);
        let mut advance = false;
        match self.focus {
            Stop::Item(index) => {
                let id = self.items.get(index).map(|item| item.id);
                match self.items.get_mut(index).map(|item| &mut item.kind) {
                    Some(Kind::Action) if enter => {
                        return id.map_or(FormEvent::None, FormEvent::Action)
                    }
                    Some(Kind::Toggle(on)) if enter || space => *on = !*on,
                    Some(Kind::Choice { options, selected }) => {
                        let len = options.len().max(1);
                        match key.code {
                            KeyCode::Right | KeyCode::Char(' ') => {
                                *selected = (*selected + 1) % len
                            }
                            KeyCode::Left => *selected = (*selected + len - 1) % len,
                            KeyCode::Enter => advance = true,
                            _ => {}
                        }
                    }
                    Some(Kind::Text { .. }) if enter => advance = true,
                    Some(Kind::Text { field, .. }) => field.input(key),
                    _ => {}
                }
            }
            Stop::RowName(..) | Stop::RowValue(..) if enter => advance = true,
            Stop::RowName(item, row) => {
                if let Some(row) = self.row_mut(item, row) {
                    row.name.input(key);
                }
            }
            Stop::RowValue(item, row) => {
                if let Some(row) = self.row_mut(item, row) {
                    row.value.input(key);
                }
            }
            Stop::RowSecret(item, row) if enter || space => {
                if let Some(row) = self.row_mut(item, row) {
                    let secret = !row.value.is_secret();
                    row.value.set_secret(secret);
                }
            }
            Stop::AddRow(item) if enter => {
                if let Some(FormItem {
                    kind: Kind::Rows { rows, .. },
                    ..
                }) = self.items.get_mut(item)
                {
                    rows.push(Row {
                        name: TextField::new("", false),
                        value: TextField::new("", false),
                    });
                    self.focus = Stop::RowName(item, rows.len() - 1);
                }
            }
            Stop::Advanced if enter || space => self.advanced_open = !self.advanced_open,
            _ => {}
        }
        if advance {
            self.focus_stop(&stops, step(1));
        }
        FormEvent::None
    }

    /// Pasted text goes to the focused text field, if one has the focus.
    pub(crate) fn handle_paste(&mut self, pasted: &str) -> bool {
        let field = match self.focus {
            Stop::Item(index) => match self.items.get_mut(index).map(|item| &mut item.kind) {
                Some(Kind::Text { field, .. }) => Some(field),
                _ => None,
            },
            Stop::RowName(item, row) => self.row_mut(item, row).map(|row| &mut row.name),
            Stop::RowValue(item, row) => self.row_mut(item, row).map(|row| &mut row.value),
            _ => None,
        };
        field.map(|field| field.paste(pasted)).is_some()
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

    /// The rows of `id` that have a name.
    pub(crate) fn rows(&self, id: FieldId) -> Vec<RowValue> {
        let Some(Kind::Rows { rows, .. }) = self.item(id).map(|item| &item.kind) else {
            return Vec::new();
        };
        rows.iter()
            .filter(|row| !row.name.text().trim().is_empty())
            .map(|row| RowValue {
                name: row.name.text().trim().to_string(),
                value: row.value.text().to_string(),
                secret: row.value.is_secret(),
            })
            .collect()
    }

    /// Says what is wrong with `id` under it, and moves the focus there.
    pub(crate) fn set_error(&mut self, id: FieldId, error: impl Into<String>) {
        let Some(index) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        self.items[index].error = Some(error.into());
        if self.items[index].advanced {
            self.advanced_open = true;
        }
        if let Some(stop) = self
            .stops()
            .into_iter()
            .find(|stop| stop_item(*stop) == Some(index))
        {
            self.focus = stop;
        }
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

    fn row_mut(&mut self, item: usize, row: usize) -> Option<&mut Row> {
        match self.items.get_mut(item).map(|item| &mut item.kind) {
            Some(Kind::Rows { rows, .. }) => rows.get_mut(row),
            _ => None,
        }
    }

    fn focus_stop(&mut self, stops: &[Stop], index: usize) {
        if let Some(stop) = stops.get(index) {
            self.focus = *stop;
        }
    }

    fn visible(&self, item: &FormItem) -> bool {
        (!item.advanced || self.advanced_open)
            && item
                .shown_if
                .is_none_or(|(choice, option)| self.choice(choice) == option)
    }

    /// The stops in order: the items shown, the "Advanced" switch before
    /// the first advanced one.
    fn stops(&self) -> Vec<Stop> {
        let mut stops = Vec::new();
        let mut advanced_seen = false;
        for (index, item) in self.items.iter().enumerate() {
            if item.advanced && !advanced_seen {
                advanced_seen = true;
                stops.push(Stop::Advanced);
            }
            if !self.visible(item) {
                continue;
            }
            match &item.kind {
                Kind::Rows {
                    rows,
                    secret_column,
                    ..
                } => {
                    for row in 0..rows.len() {
                        stops.push(Stop::RowName(index, row));
                        stops.push(Stop::RowValue(index, row));
                        if *secret_column {
                            stops.push(Stop::RowSecret(index, row));
                        }
                    }
                    stops.push(Stop::AddRow(index));
                }
                _ => stops.push(Stop::Item(index)),
            }
        }
        stops
    }
}

/// The item a stop belongs to.
fn stop_item(stop: Stop) -> Option<usize> {
    match stop {
        Stop::Item(item)
        | Stop::RowName(item, _)
        | Stop::RowValue(item, _)
        | Stop::RowSecret(item, _)
        | Stop::AddRow(item) => Some(item),
        Stop::Advanced => None,
    }
}

#[cfg(test)]
mod tests;
