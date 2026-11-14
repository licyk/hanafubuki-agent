// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The tabs (Codex's sections): a group's sections, each setting a row,
//! its collections and its pages; and `/`'s search over every tab. A
//! boolean saves at once, a choice opens a list, a text, a number or a
//! path is edited in its row, a list of texts opens a list, a member of a
//! commit group opens a form of its group, and `r` resets.

use hanafubuki_agent::{
    SettingKind, SettingSource, SettingSpec, SettingValue, SettingsCatalog, SettingsGroup,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

use super::collection::{self, CollectionPage};
use super::editor::{ChoicePage, LineEditor, ListPage, ListState};
use super::form::FormPage;
use super::registry::{self, Attach, PageOverride};
use super::render::{Row, RowStyle};
use super::values::{self, group_label, value_text, GROUPS};
use super::{Effect, Page};
use crate::bottom_pane::TextField;

#[derive(Default)]
pub(super) struct MainPage {
    /// The tab shown, among those [`tabs`] has.
    pub(super) tab: usize,
    pub(super) list: ListState,
    pub(super) search: Option<Search>,
    pub(super) editor: Option<LineEditor>,
}

pub(super) struct Search {
    pub(super) query: TextField,
    /// Keys type into the query.
    pub(super) editing: bool,
}

/// What a row of a tab is.
#[derive(Clone, Copy)]
pub(super) enum MainRow {
    Heading(usize),
    Setting { section: usize, index: usize },
    Collection(usize),
    Record { collection: usize, record: usize },
    Page(&'static PageOverride),
}

/// The tabs that have rows, in their order.
pub(super) fn tabs(catalog: &SettingsCatalog) -> Vec<SettingsGroup> {
    GROUPS
        .into_iter()
        .filter(|group| !group_rows(catalog, *group).is_empty())
        .collect()
}

/// The rows of `group`: its sections by their order, each setting shown
/// by its condition (a section of more than one under its heading), then
/// its collections and its pages.
pub(super) fn group_rows(catalog: &SettingsCatalog, group: SettingsGroup) -> Vec<MainRow> {
    let mut sections = catalog
        .sections
        .iter()
        .enumerate()
        .filter(|(_, section)| section.group == group)
        .collect::<Vec<_>>();
    sections.sort_by_key(|(_, section)| section.order);
    let mut rows = Vec::new();
    for (section_index, section) in sections {
        if section.settings.len() > 1 {
            rows.push(MainRow::Heading(section_index));
        }
        for (index, spec) in section.settings.iter().enumerate() {
            if values::holds(spec.visible_if.as_ref(), |key| {
                setting_value(catalog, key).cloned()
            }) {
                rows.push(MainRow::Setting {
                    section: section_index,
                    index,
                });
            }
        }
    }
    rows.extend(
        catalog
            .collections
            .iter()
            .enumerate()
            .filter(|(_, collection)| collection.group == group)
            .map(|(index, _)| MainRow::Collection(index)),
    );
    rows.extend(registry::row_pages(group).map(MainRow::Page));
    rows
}

/// The value of the setting `key`, as the catalog resolves it.
pub(super) fn setting_value<'a>(
    catalog: &'a SettingsCatalog,
    key: &str,
) -> Option<&'a SettingValue> {
    state(catalog, key).and_then(|state| state.value.as_ref())
}

fn state<'a>(
    catalog: &'a SettingsCatalog,
    key: &str,
) -> Option<&'a hanafubuki_agent::SettingState> {
    catalog
        .sections
        .iter()
        .flat_map(|section| &section.values)
        .find(|state| state.key == key)
}

impl MainPage {
    /// The rows shown: the tab's, or the search's across every tab.
    pub(super) fn rows(&self, catalog: &SettingsCatalog) -> Vec<MainRow> {
        match &self.search {
            Some(search) => search_rows(catalog, &search.query.text().to_lowercase()),
            None => {
                let tabs = tabs(catalog);
                tabs.get(self.tab.min(tabs.len().saturating_sub(1)))
                    .map(|group| group_rows(catalog, *group))
                    .unwrap_or_default()
            }
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, catalog: &SettingsCatalog) -> Effect {
        let rows = self.rows(catalog);
        if let Some(editor) = self.editor.as_mut() {
            return match editor.handle_key(key) {
                Some(true) => match selected_spec(catalog, &rows, self.list.selected) {
                    Some(spec) => editor.submit(spec, None),
                    None => Effect::None,
                },
                Some(false) => {
                    self.editor = None;
                    Effect::None
                }
                None => Effect::None,
            };
        }
        if let Some(search) = self.search.as_mut().filter(|search| search.editing) {
            match key.code {
                KeyCode::Esc => self.search = None,
                KeyCode::Enter | KeyCode::Down => search.editing = false,
                _ => search.query.input(key),
            }
            self.list = ListState::first(&self.rows(catalog), selectable);
            return Effect::None;
        }
        match key.code {
            KeyCode::Esc if self.search.is_some() => {
                self.search = None;
                self.list = ListState::default();
            }
            KeyCode::Esc => return Effect::Close,
            KeyCode::Left | KeyCode::Right if self.search.is_none() => {
                let count = tabs(catalog).len().max(1);
                self.tab = match key.code {
                    KeyCode::Left => (self.tab + count - 1) % count,
                    _ => (self.tab + 1) % count,
                };
                self.list = ListState::first(&self.rows(catalog), selectable);
            }
            KeyCode::Char('/') if key.modifiers == KeyModifiers::NONE => {
                self.search = Some(Search {
                    query: TextField::new("", false),
                    editing: true,
                });
                self.list = ListState::first(&self.rows(catalog), selectable);
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => {
                if let Some(MainRow::Setting { section, index }) = rows.get(self.list.selected) {
                    let spec = &catalog.sections[*section].settings[*index];
                    return Effect::Reset(spec.key.clone());
                }
            }
            KeyCode::Char(' ') => {
                if let Some(spec) = selected_spec(catalog, &rows, self.list.selected) {
                    if spec.kind == SettingKind::Boolean {
                        return toggle(spec, setting_value(catalog, &spec.key));
                    }
                }
            }
            KeyCode::Enter => return self.activate(catalog, &rows),
            _ => self.list.navigate(key, &rows, selectable),
        }
        Effect::None
    }

    /// Enter on the selected row.
    pub(super) fn activate(&mut self, catalog: &SettingsCatalog, rows: &[MainRow]) -> Effect {
        match rows.get(self.list.selected).copied() {
            Some(MainRow::Setting { section, index }) => {
                let section = &catalog.sections[section];
                let spec = &section.settings[index];
                if let Some(page) = registry::page_of_setting(&spec.key) {
                    return Effect::Open(page, None);
                }
                if let Some(group) = spec.commit_group.as_deref() {
                    return Effect::Push(Box::new(Page::Form(FormPage::group(section, group))));
                }
                let value = setting_value(catalog, &spec.key);
                match &spec.kind {
                    SettingKind::Boolean => toggle(spec, value),
                    SettingKind::Choice { .. } | SettingKind::ChoiceOrText { .. } => {
                        Effect::Push(Box::new(Page::Choice(ChoicePage::edit(spec, value, 0))))
                    }
                    SettingKind::TextList { .. } => {
                        Effect::Push(Box::new(Page::List(ListPage::new(spec.clone(), 0))))
                    }
                    _ => {
                        self.editor = Some(LineEditor::new(spec, value));
                        Effect::None
                    }
                }
            }
            Some(MainRow::Collection(index)) => {
                let collection = &catalog.collections[index];
                Effect::Push(Box::new(match collection::singleton(collection) {
                    Some(record) => Page::Form(FormPage::record(collection, record)),
                    None => Page::Collection(CollectionPage::new(collection)),
                }))
            }
            Some(MainRow::Record { collection, record }) => {
                let collection = &catalog.collections[collection];
                Effect::Push(Box::new(Page::Form(FormPage::record(
                    collection,
                    &collection.records[record],
                ))))
            }
            Some(MainRow::Page(page)) => Effect::Open(page, None),
            Some(MainRow::Heading(_)) | None => Effect::None,
        }
    }

    /// The selected setting typed in its row, whatever its kind offers.
    pub(super) fn type_selected(&mut self, catalog: &SettingsCatalog) {
        if let Some(spec) = selected_spec(catalog, &self.rows(catalog), self.list.selected) {
            self.editor = Some(LineEditor::new(spec, setting_value(catalog, &spec.key)));
        }
    }

    /// The tab at `tab`, as a click on it shows it.
    pub(super) fn show_tab(&mut self, catalog: &SettingsCatalog, tab: usize) {
        self.tab = tab;
        self.search = None;
        self.editor = None;
        self.list = ListState::first(&self.rows(catalog), selectable);
    }

    pub(super) fn paste(&mut self, text: &str) {
        if let Some(editor) = self.editor.as_mut() {
            editor.paste(text);
        } else if let Some(search) = self.search.as_mut().filter(|search| search.editing) {
            search.query.paste(text);
        }
    }

    /// The change made in the row's editor was saved.
    pub(super) fn saved(&mut self) {
        self.editor = None;
    }

    /// It was refused: the editor stays with the reason.
    pub(super) fn refused(&mut self, error: &str) {
        if let Some(editor) = self.editor.as_mut() {
            editor.refused(error);
        }
    }

    /// `/config TARGET`: the tab a group, a section, a collection or a page
    /// is in, and the collection or the page opened; `None` when nothing
    /// is named so.
    pub(super) fn open_target(
        &mut self,
        catalog: &SettingsCatalog,
        target: &str,
    ) -> Option<Effect> {
        let target = target.trim().to_lowercase();
        let tabs = tabs(catalog);
        let found = tabs.iter().enumerate().find_map(|(tab, group)| {
            if group_label(*group).to_lowercase() == target {
                return Some((tab, None));
            }
            let rows = group_rows(catalog, *group);
            rows.iter()
                .position(|row| match row {
                    MainRow::Heading(section) | MainRow::Setting { section, .. } => {
                        catalog.sections[*section].id.to_lowercase() == target
                    }
                    MainRow::Collection(index) => {
                        catalog.collections[*index].id.to_lowercase() == target
                    }
                    MainRow::Page(page) => page.id == target,
                    MainRow::Record { .. } => false,
                })
                .map(|position| (tab, Some((rows, position))))
        });
        let page = registry::page(&target).filter(|page| matches!(page.attach, Attach::Setting(_)));
        let (tab, rows) = match (found, page) {
            (Some(found), _) => found,
            (None, Some(page)) => {
                let Attach::Setting(key) = page.attach else {
                    return None;
                };
                let tab = tabs.iter().position(|group| {
                    group_rows(catalog, *group).iter().any(|row| {
                        matches!(row, MainRow::Setting { section, index }
                            if catalog.sections[*section].settings[*index].key == key)
                    })
                })?;
                self.tab = tab;
                return Some(Effect::Open(page, None));
            }
            (None, None) => return None,
        };
        self.tab = tab;
        let Some((rows, position)) = rows else {
            self.list = ListState::first(&self.rows(catalog), selectable);
            return Some(Effect::None);
        };
        let position = (position..rows.len())
            .find(|index| selectable(&rows[*index]))
            .unwrap_or(position);
        self.list = ListState::at(position);
        Some(match rows[position] {
            MainRow::Collection(_) | MainRow::Page(_) => self.activate(catalog, &rows),
            _ => Effect::None,
        })
    }

    /// The rows as they are drawn.
    pub(super) fn view<'a>(
        &'a self,
        catalog: &'a SettingsCatalog,
        rows: &[MainRow],
    ) -> Vec<Row<'a>> {
        let searching = self.search.is_some();
        rows.iter()
            .enumerate()
            .map(|(index, row)| {
                let editing = (index == self.list.selected)
                    .then_some(self.editor.as_ref())
                    .flatten();
                view_row(catalog, row, searching, editing)
            })
            .collect()
    }
}

pub(super) fn selectable(row: &MainRow) -> bool {
    !matches!(row, MainRow::Heading(_))
}

fn selected_spec<'a>(
    catalog: &'a SettingsCatalog,
    rows: &[MainRow],
    selected: usize,
) -> Option<&'a SettingSpec> {
    match rows.get(selected)? {
        MainRow::Setting { section, index } => Some(&catalog.sections[*section].settings[*index]),
        _ => None,
    }
}

/// A boolean the other way, saved at once.
fn toggle(spec: &SettingSpec, value: Option<&SettingValue>) -> Effect {
    let on = matches!(value, Some(SettingValue::Boolean(true)));
    Effect::Edit {
        key: spec.key.clone(),
        value: Value::Bool(!on),
        at: None,
        pop: false,
    }
}

/// Every setting, collection, record and page whose label, key, value,
/// keywords or summary has `needle`.
fn search_rows(catalog: &SettingsCatalog, needle: &str) -> Vec<MainRow> {
    let matches = |texts: &[&str]| {
        texts
            .iter()
            .any(|text| text.to_lowercase().contains(needle))
    };
    let mut rows = Vec::new();
    for group in GROUPS {
        for row in group_rows(catalog, group) {
            let found = match &row {
                MainRow::Heading(_) | MainRow::Record { .. } => false,
                MainRow::Setting { section, index } => {
                    let spec = &catalog.sections[*section].settings[*index];
                    let value = value_text(spec, setting_value(catalog, &spec.key), false);
                    let mut texts = vec![spec.label.as_str(), &spec.key, &value, &spec.summary];
                    texts.extend(spec.keywords.iter().map(String::as_str));
                    matches(&texts)
                }
                MainRow::Collection(index) => matches(&[&catalog.collections[*index].title]),
                MainRow::Page(page) => match page.attach {
                    Attach::Row { label, summary, .. } => matches(&[label, summary, page.id]),
                    Attach::Setting(_) | Attach::Record { .. } | Attach::RecordField { .. } => {
                        false
                    }
                },
            };
            if found {
                rows.push(row);
            }
            if let MainRow::Collection(collection) = row {
                let records = &catalog.collections[collection].records;
                rows.extend(
                    records
                        .iter()
                        .enumerate()
                        .filter(|(_, record)| {
                            matches(&[&record.label, &record.id, &record.summary])
                        })
                        .map(|(record, _)| MainRow::Record { collection, record }),
                );
            }
        }
    }
    rows
}

fn view_row<'a>(
    catalog: &'a SettingsCatalog,
    row: &MainRow,
    searching: bool,
    editing: Option<&'a LineEditor>,
) -> Row<'a> {
    match *row {
        MainRow::Heading(section) => Row::heading(&catalog.sections[section].title),
        MainRow::Setting { section, index } => {
            let section = &catalog.sections[section];
            let spec = &section.settings[index];
            let state = state(catalog, &spec.key);
            let value = state.and_then(|state| state.value.as_ref());
            let page = registry::page_of_setting(&spec.key);
            let mut row = Row::setting(
                spec,
                value,
                false,
                page.is_some() || spec.commit_group.is_some(),
            );
            row.source = state.and_then(|state| values::source_text(&state.source));
            if let Some(issue) = state.and_then(|state| state.issue.as_ref()) {
                row.description = format!("⚠ {issue}");
                row.style = RowStyle::Issue;
            }
            if searching {
                row.description = format!("{} · {}", group_label(section.group), section.title);
            }
            if state.is_some_and(|state| state.source == SettingSource::User) {
                row.hint.push_str(" · r reset");
            }
            row.editor = editing;
            row
        }
        MainRow::Collection(index) => {
            let collection = &catalog.collections[index];
            let row = collection::summary_row(collection);
            searched(row, searching, collection.group, &collection.title)
        }
        MainRow::Record { collection, record } => {
            let collection = &catalog.collections[collection];
            let row = collection::record_row(&collection.records[record]);
            searched(row, true, collection.group, &collection.title)
        }
        MainRow::Page(page) => {
            let Attach::Row {
                group,
                label,
                summary,
            } = page.attach
            else {
                return Row::heading(page.id);
            };
            let row = Row::open(label, "Open ›", summary);
            searched(row, searching, group, label)
        }
    }
}

fn searched<'a>(mut row: Row<'a>, searching: bool, group: SettingsGroup, title: &str) -> Row<'a> {
    if searching {
        row.description = format!("{} · {title}", group_label(group));
    }
    row
}
