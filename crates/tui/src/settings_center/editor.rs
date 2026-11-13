// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The editors: a row's one-line editor for a text, a number or a path;
//! Codex's child picker for a choice (`(current)`, disabled values with
//! their reasons), for a choice-or-text (its values, Default, and Custom,
//! which types one in the row's editor) and for a confirmation; and a
//! list of texts, each item a row, added, edited and removed one at a time.

use hanafubuki_agent::{SettingKind, SettingSpec, SettingValue, SettingsActionRequest};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

use super::render::{Row, RowStyle};
use super::values::editable_text;
use super::Effect;
use crate::bottom_pane::TextField;
use crate::model_facts::{effort_options, EffortValue};

/// The selected row of a list, and the first one shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ListState {
    pub(super) selected: usize,
    pub(super) scroll: usize,
}

impl ListState {
    pub(super) fn at(selected: usize) -> Self {
        Self {
            selected,
            scroll: 0,
        }
    }

    /// The first row that can be selected.
    pub(super) fn first<T>(rows: &[T], selectable: impl Fn(&T) -> bool) -> Self {
        Self::at(rows.iter().position(selectable).unwrap_or(0))
    }

    /// Up, Down, Page Up, Page Down, Home and End, over the rows that can
    /// be selected.
    pub(super) fn navigate<T>(
        &mut self,
        key: KeyEvent,
        rows: &[T],
        selectable: impl Fn(&T) -> bool,
    ) {
        let candidates = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| selectable(row))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let Some(&last) = candidates.last() else {
            return;
        };
        let current = candidates
            .iter()
            .position(|index| *index >= self.selected)
            .unwrap_or(candidates.len() - 1);
        let step = |by: isize| {
            let position = current.saturating_add_signed(by).min(candidates.len() - 1);
            candidates[position]
        };
        self.selected = match key.code {
            KeyCode::Up | KeyCode::Char('k') => step(-1),
            KeyCode::Down | KeyCode::Char('j') => step(1),
            KeyCode::PageUp => step(-10),
            KeyCode::PageDown => step(10),
            KeyCode::Home => candidates[0],
            KeyCode::End => last,
            _ => return,
        };
    }

    /// The scroll that keeps the selected row among `height` rows.
    pub(super) fn keep_visible(&mut self, height: usize, count: usize) {
        let height = height.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
        self.scroll = self.scroll.min(count.saturating_sub(height));
    }
}

/// A text, a number or a path edited in its row. Enter checks it as the
/// write will; a refusal stays in the editor with its reason.
pub(super) struct LineEditor {
    field: TextField,
    pub(super) error: Option<String>,
    /// Sent, waiting for the answer.
    pub(super) waiting: bool,
}

impl LineEditor {
    pub(super) fn new(spec: &SettingSpec, value: Option<&SettingValue>) -> Self {
        let secret = matches!(spec.kind, SettingKind::Text { secret: true, .. });
        let text = if secret {
            String::new()
        } else {
            editable_text(value)
        };
        Self {
            field: TextField::new(&text, secret),
            error: None,
            waiting: false,
        }
    }

    pub(super) fn field(&self) -> &TextField {
        &self.field
    }

    /// `Some(true)` on Enter, `Some(false)` on Esc; other keys edit.
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Option<bool> {
        match key.code {
            KeyCode::Enter => Some(true),
            KeyCode::Esc => Some(false),
            _ => {
                self.field.input(key);
                self.error = None;
                None
            }
        }
    }

    pub(super) fn paste(&mut self, text: &str) {
        self.field.paste(text);
        self.error = None;
    }

    /// The edit of `spec` as a change of the page `at` (the page itself
    /// when `None`), or why it cannot be.
    pub(super) fn submit(&mut self, spec: &SettingSpec, at: Option<usize>) -> Effect {
        match typed(spec, self.field.text()).and_then(|value| {
            spec.check(&value)?;
            Ok(value)
        }) {
            Ok(value) => {
                self.waiting = true;
                Effect::Edit {
                    key: spec.key.clone(),
                    value,
                    at,
                    pop: false,
                }
            }
            Err(error) => {
                self.error = Some(error);
                Effect::None
            }
        }
    }

    pub(super) fn refused(&mut self, error: &str) {
        self.waiting = false;
        self.error = Some(error.to_string());
    }
}

/// The text typed as a value of `spec`'s kind: empty is `null` for a
/// nullable text.
fn typed(spec: &SettingSpec, text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    match &spec.kind {
        SettingKind::Integer { .. } => trimmed
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| "Type an integer.".to_string()),
        SettingKind::Number { .. } => trimmed
            .parse::<f64>()
            .map(Value::from)
            .map_err(|_| "Type a number.".to_string()),
        SettingKind::Text { nullable: true, .. }
        | SettingKind::ChoiceOrText { nullable: true, .. }
            if text.is_empty() =>
        {
            Ok(Value::Null)
        }
        _ => Ok(Value::String(text.to_string())),
    }
}

/// Codex's child picker: a choice's values, or a confirmation.
pub(super) struct ChoicePage {
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) options: Vec<ChoiceOption>,
    pub(super) list: ListState,
    purpose: Purpose,
}

pub(super) struct ChoiceOption {
    pub(super) label: String,
    /// The value it sets; `None` for a value typed in the field's row.
    pub(super) value: Option<Value>,
    /// Why it cannot be chosen, or what it is.
    pub(super) disabled: Option<String>,
    pub(super) description: Option<String>,
    pub(super) current: bool,
}

enum Purpose {
    /// The field `key` of the page at `at` takes the value chosen.
    Edit { key: String, at: usize },
    /// The action runs when the second option is chosen.
    Confirm(Box<SettingsActionRequest>),
}

impl ChoicePage {
    /// The values of `spec`, the current one selected, for the field of
    /// the page at `at`: a choice's, or the efforts a choice-or-text
    /// offers (`effort_options`).
    pub(super) fn edit(spec: &SettingSpec, value: Option<&SettingValue>, at: usize) -> Self {
        let current = match value {
            Some(SettingValue::Text(text)) => Some(text.as_str()),
            _ => None,
        };
        let options: Vec<ChoiceOption> = match &spec.kind {
            SettingKind::Choice { values } => values
                .iter()
                .map(|choice| ChoiceOption {
                    label: choice.label.clone(),
                    value: Some(Value::String(choice.value.clone())),
                    disabled: choice.disabled.clone(),
                    description: None,
                    current: current == Some(choice.value.as_str()),
                })
                .collect(),
            SettingKind::ChoiceOrText {
                values, nullable, ..
            } => effort_options(values, current)
                .into_iter()
                .filter(|option| *nullable || option.value != EffortValue::Default)
                .map(|option| ChoiceOption {
                    value: match option.value {
                        EffortValue::Level(level) => Some(Value::String(level)),
                        EffortValue::Default => Some(Value::Null),
                        EffortValue::Custom => None,
                    },
                    label: option.label,
                    disabled: None,
                    description: option.description.map(str::to_string),
                    current: option.current,
                })
                .collect(),
            _ => unreachable!("a choice page is opened for a choice"),
        };
        let selected = options
            .iter()
            .position(|option| option.current)
            .or_else(|| options.iter().position(|option| option.disabled.is_none()))
            .unwrap_or(0);
        Self {
            title: spec.label.clone(),
            subtitle: spec.summary.clone(),
            options,
            list: ListState::at(selected),
            purpose: Purpose::Edit {
                key: spec.key.clone(),
                at,
            },
        }
    }

    /// Asks before `request` runs: Cancel, selected, or `label`.
    pub(super) fn confirm(label: &str, subject: &str, request: SettingsActionRequest) -> Self {
        let option = |label: &str| ChoiceOption {
            label: label.to_string(),
            value: Some(Value::Null),
            disabled: None,
            description: None,
            current: false,
        };
        Self {
            title: format!("{label}?"),
            subtitle: subject.to_string(),
            options: vec![option("Cancel"), option(label)],
            list: ListState::default(),
            purpose: Purpose::Confirm(Box::new(request)),
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Esc => Effect::Pop,
            KeyCode::Enter | KeyCode::Char(' ') => self.choose(),
            KeyCode::Char(digit @ '1'..='9') if key.modifiers == KeyModifiers::NONE => {
                let index = digit as usize - '1' as usize;
                if index < self.options.len() {
                    self.list.selected = index;
                    return self.choose();
                }
                Effect::None
            }
            _ => {
                self.list
                    .navigate(key, &self.options, |option| option.disabled.is_none());
                Effect::None
            }
        }
    }

    /// The selected option taken.
    pub(super) fn choose(&mut self) -> Effect {
        let Some(option) = self.options.get(self.list.selected) else {
            return Effect::None;
        };
        if option.disabled.is_some() {
            return Effect::None;
        }
        match (&self.purpose, &option.value) {
            (Purpose::Edit { key, at }, Some(value)) => Effect::Edit {
                key: key.clone(),
                value: value.clone(),
                at: Some(*at),
                pop: true,
            },
            (Purpose::Edit { at, .. }, None) => Effect::Type(*at),
            (Purpose::Confirm(_), _) if self.list.selected == 0 => Effect::Pop,
            (Purpose::Confirm(request), _) => Effect::Run {
                request: (**request).clone(),
                pop: true,
            },
        }
    }

    pub(super) fn view(&self) -> Vec<Row<'static>> {
        self.options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let mut label = format!("{}. {}", index + 1, option.label);
                if option.current {
                    label.push_str(" (current)");
                }
                let description = option.disabled.as_ref().or(option.description.as_ref());
                let mut row = Row::open(&label, "", description.map_or("", String::as_str));
                row.hint = if option.value.is_some() {
                    "enter select"
                } else {
                    "enter type"
                }
                .to_string();
                if option.disabled.is_some() {
                    row.style = RowStyle::Disabled;
                }
                row
            })
            .collect()
    }
}

/// A list of texts: each item a row, Enter edits it, `d` removes it, and
/// the last row adds one. Each change is the field's new value at once.
pub(super) struct ListPage {
    pub(super) spec: SettingSpec,
    /// The page whose field it is.
    pub(super) at: usize,
    pub(super) list: ListState,
    /// The item being edited, `None` for a new one.
    pub(super) editor: Option<(Option<usize>, LineEditor)>,
}

impl ListPage {
    pub(super) fn new(spec: SettingSpec, at: usize) -> Self {
        Self {
            spec,
            at,
            list: ListState::default(),
            editor: None,
        }
    }

    /// `items` is the field's value as its page shows it now.
    pub(super) fn handle_key_with(&mut self, key: KeyEvent, items: &[String]) -> Effect {
        if let Some((index, editor)) = self.editor.as_mut() {
            return match editor.handle_key(key) {
                Some(true) => {
                    let text = editor.field().text().trim().to_string();
                    let mut changed = items.to_vec();
                    match (*index, text.is_empty()) {
                        (Some(index), true) => {
                            changed.remove(index);
                        }
                        (Some(index), false) => changed[index] = text,
                        (None, true) => {
                            self.editor = None;
                            return Effect::None;
                        }
                        (None, false) => changed.push(text),
                    }
                    self.editor = None;
                    self.change(changed)
                }
                Some(false) => {
                    self.editor = None;
                    Effect::None
                }
                None => Effect::None,
            };
        }
        let count = items.len() + 1;
        match key.code {
            KeyCode::Esc => Effect::Pop,
            KeyCode::Enter => {
                let index = (self.list.selected < items.len()).then_some(self.list.selected);
                let text = index.map(|index| SettingValue::Text(items[index].clone()));
                self.editor = Some((index, LineEditor::new(&self.item_spec(), text.as_ref())));
                Effect::None
            }
            KeyCode::Char('d') if self.list.selected < items.len() => {
                let mut changed = items.to_vec();
                changed.remove(self.list.selected);
                self.change(changed)
            }
            _ => {
                let rows = vec![(); count];
                self.list.navigate(key, &rows, |_| true);
                Effect::None
            }
        }
    }

    pub(super) fn paste(&mut self, text: &str) {
        if let Some((_, editor)) = self.editor.as_mut() {
            editor.paste(text);
        }
    }

    fn change(&self, items: Vec<String>) -> Effect {
        Effect::Edit {
            key: self.spec.key.clone(),
            value: Value::from(items),
            at: Some(self.at),
            pop: false,
        }
    }

    /// An item, as a plain text.
    fn item_spec(&self) -> SettingSpec {
        SettingSpec::new(
            self.spec.key.clone(),
            self.spec.label.clone(),
            SettingKind::Text {
                maximum_chars: None,
                secret: false,
                nullable: false,
            },
        )
    }

    pub(super) fn view<'a>(&'a self, items: &[String]) -> Vec<Row<'a>> {
        let editing = |index: Option<usize>| {
            self.editor
                .as_ref()
                .filter(|(edited, _)| *edited == index)
                .map(|(_, editor)| editor)
        };
        let mut rows = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let mut row = Row::open(&format!("{}.", index + 1), item, "");
                row.hint = "enter edit · d remove".to_string();
                row.editor = editing(Some(index));
                row
            })
            .collect::<Vec<_>>();
        let mut add = Row::open("+ Add an item", "", "");
        add.hint = "enter add".to_string();
        add.editor = editing(None);
        rows.push(add);
        rows
    }
}
