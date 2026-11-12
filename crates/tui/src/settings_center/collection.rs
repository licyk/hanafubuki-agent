// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A collection's page: why its records could not all be read, its
//! records with their state and summary, adding one (`a`), deleting the
//! selected one (`d`), and the collection's own actions.

use hanafubuki_agent::{
    SettingsActionRequest, SettingsActionScope, SettingsCatalog, SettingsCollection, SettingsRecord,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::editor::{ChoicePage, ListState};
use super::form::FormPage;
use super::render::{Row, RowStyle};
use super::{Effect, Page};

const ADD: &str = "add";
const SAVE: &str = "save";
const DELETE: &str = "delete";

pub(super) struct CollectionPage {
    id: String,
    pub(super) list: ListState,
}

/// What a row of the page is.
#[derive(Clone, Copy)]
enum Line {
    Issue,
    Record(usize),
    /// A collection's action (`add` among them), by its place in the
    /// collection's actions.
    Action(usize),
}

/// The record a collection that holds one and adds none opens as (the
/// model policy).
pub(super) fn singleton(collection: &SettingsCollection) -> Option<&SettingsRecord> {
    match collection.records.as_slice() {
        [record] if !has(collection, ADD) => Some(record),
        _ => None,
    }
}

/// Whether the collection has the action `id`.
pub(super) fn has(collection: &SettingsCollection, id: &str) -> bool {
    collection.actions.iter().any(|action| action.id == id)
}

/// Whether the collection's records can be written.
pub(super) fn writable(collection: &SettingsCollection) -> bool {
    has(collection, SAVE)
}

/// The tab's row of a collection: how many records, or the one it holds.
pub(super) fn summary_row(collection: &SettingsCollection) -> Row<'static> {
    let value = match singleton(collection) {
        Some(record) if !record.summary.is_empty() => format!("{} ›", record.summary),
        Some(record) => format!("{} ›", record.label),
        None => match collection.records.len() {
            1 => "1 entry ›".to_string(),
            count => format!("{count} entries ›"),
        },
    };
    let description = match collection.issues.first() {
        Some(issue) => format!("⚠ {issue}"),
        None => collection
            .records
            .iter()
            .map(|record| record.label.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    };
    let mut row = Row::open(&collection.title, &value, &description);
    if !collection.issues.is_empty() {
        row.style = RowStyle::Issue;
    }
    row
}

/// A record as a row: its label, its state, its summary or its issue.
pub(super) fn record_row(record: &SettingsRecord) -> Row<'static> {
    let value = match record.state.as_deref() {
        Some(state) => format!("{state} ›"),
        None => "›".to_string(),
    };
    let (description, style) = match &record.issue {
        Some(issue) => (format!("⚠ {issue}"), RowStyle::Issue),
        None => (record.summary.clone(), RowStyle::Plain),
    };
    let mut row = Row::open(&record.label, &value, &description);
    row.style = style;
    row
}

impl CollectionPage {
    pub(super) fn new(collection: &SettingsCollection) -> Self {
        let mut page = Self {
            id: collection.id.clone(),
            list: ListState::default(),
        };
        page.list = ListState::first(&page.lines(collection), |line| !matches!(line, Line::Issue));
        page
    }

    pub(super) fn collection<'a>(
        &self,
        catalog: &'a SettingsCatalog,
    ) -> Option<&'a SettingsCollection> {
        catalog
            .collections
            .iter()
            .find(|collection| collection.id == self.id)
    }

    /// Selects the record `id`, as an `add` made it.
    pub(super) fn select_record(&mut self, catalog: Option<&SettingsCatalog>, id: &str) {
        let Some(collection) = catalog.and_then(|catalog| self.collection(catalog)) else {
            return;
        };
        let lines = self.lines(collection);
        if let Some(position) = lines.iter().position(
            |line| matches!(line, Line::Record(index) if collection.records[*index].id == id),
        ) {
            self.list = ListState::at(position);
        }
    }

    fn lines(&self, collection: &SettingsCollection) -> Vec<Line> {
        let mut lines = vec![Line::Issue; collection.issues.len()];
        lines.extend((0..collection.records.len()).map(Line::Record));
        lines.extend(
            collection
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| action.scope == SettingsActionScope::Collection)
                .map(|(index, _)| Line::Action(index)),
        );
        lines
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, catalog: &SettingsCatalog) -> Effect {
        let Some(collection) = self.collection(catalog) else {
            return Effect::Pop;
        };
        let lines = self.lines(collection);
        let selected = lines.get(self.list.selected).copied();
        let plain = key.modifiers == KeyModifiers::NONE;
        match key.code {
            KeyCode::Esc => Effect::Pop,
            KeyCode::Enter => self.activate(collection, selected),
            KeyCode::Char('a') if plain && has(collection, ADD) => {
                Effect::Push(Box::new(Page::Form(FormPage::add(collection))))
            }
            KeyCode::Char('d') if plain && has(collection, DELETE) => match selected {
                Some(Line::Record(index)) => {
                    let record = &collection.records[index];
                    let action = collection.actions.iter().find(|action| action.id == DELETE);
                    let label = action.map_or("Delete", |action| action.label.as_str());
                    let request = request(collection, Some(&record.id), DELETE);
                    Effect::Push(Box::new(Page::Choice(ChoicePage::confirm(
                        label,
                        &record.label,
                        request,
                    ))))
                }
                _ => Effect::None,
            },
            _ => {
                self.list
                    .navigate(key, &lines, |line| !matches!(line, Line::Issue));
                Effect::None
            }
        }
    }

    /// Enter on the selected row.
    fn activate(&self, collection: &SettingsCollection, selected: Option<Line>) -> Effect {
        match selected {
            Some(Line::Record(index)) => Effect::Push(Box::new(Page::Form(FormPage::record(
                collection,
                &collection.records[index],
            )))),
            Some(Line::Action(index)) => {
                let action = &collection.actions[index];
                if action.id == ADD {
                    return Effect::Push(Box::new(Page::Form(FormPage::add(collection))));
                }
                let request = request(collection, None, &action.id);
                if action.confirm {
                    return Effect::Push(Box::new(Page::Choice(ChoicePage::confirm(
                        &action.label,
                        &collection.title,
                        request,
                    ))));
                }
                Effect::Run {
                    request,
                    pop: false,
                }
            }
            Some(Line::Issue) | None => Effect::None,
        }
    }

    /// Enter on the row at `index`, as a click does.
    pub(super) fn activate_at(&self, catalog: &SettingsCatalog) -> Effect {
        match self.collection(catalog) {
            Some(collection) => {
                let lines = self.lines(collection);
                self.activate(collection, lines.get(self.list.selected).copied())
            }
            None => Effect::None,
        }
    }

    pub(super) fn title(&self, catalog: &SettingsCatalog) -> String {
        self.collection(catalog)
            .map(|collection| collection.title.clone())
            .unwrap_or_default()
    }

    pub(super) fn view(&self, catalog: &SettingsCatalog) -> Vec<Row<'static>> {
        let Some(collection) = self.collection(catalog) else {
            return Vec::new();
        };
        let delete = if has(collection, DELETE) {
            " · d delete"
        } else {
            ""
        };
        let add = if has(collection, ADD) {
            " · a add"
        } else {
            ""
        };
        let mut issues = collection.issues.iter();
        self.lines(collection)
            .into_iter()
            .map(|line| match line {
                Line::Issue => {
                    let mut row = Row::open("", "", issues.next().map_or("", String::as_str));
                    row.style = RowStyle::Issue;
                    row.selectable = false;
                    row
                }
                Line::Record(index) => {
                    let mut row = record_row(&collection.records[index]);
                    row.hint = format!("enter open{delete}{add}");
                    row
                }
                Line::Action(index) => {
                    let action = &collection.actions[index];
                    let label = if action.id == ADD {
                        format!("+ {}", action.label)
                    } else {
                        action.label.clone()
                    };
                    let mut row = Row::open(&label, "", "");
                    row.hint = format!("enter run{add}");
                    row
                }
            })
            .collect()
    }
}

/// `action` on the collection, or on its record `record`, at the revision
/// it was read at.
pub(super) fn request(
    collection: &SettingsCollection,
    record: Option<&str>,
    action: &str,
) -> SettingsActionRequest {
    SettingsActionRequest {
        collection: collection.id.clone(),
        record: record.map(str::to_string),
        action: action.to_string(),
        expected_revision: collection.revision,
        values: serde_json::Map::new(),
    }
}
