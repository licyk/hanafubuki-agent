// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A form: a record of a collection, built from the collection's record
//! specs, its fields saved one at a time as the tabs' are (a field the
//! record refused stays unsaved, and goes with the next one: a transport
//! switched before its URL is typed), and its actions; a record being
//! added, or a commit group, whose fields are a draft saved together.

use hanafubuki_agent::{
    SettingChange, SettingKind, SettingSpec, SettingValue, SettingWriteOp, SettingsActionRequest,
    SettingsCatalog, SettingsCollection, SettingsRecord, SettingsSection, SettingsWrite,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Map, Value};

use super::collection::{self, request};
use super::editor::{ChoicePage, LineEditor, ListPage, ListState};
use super::registry::{self, Attach, PageOverride};
use super::render::{Row, RowStyle};
use super::values;
use super::{Effect, Page};

const ADD: &str = "add";
const SAVE: &str = "save";

pub(super) struct FormPage {
    pub(super) target: FormTarget,
    /// The values changed and not saved yet, by key.
    drafts: Map<String, Value>,
    /// The name of the record being added, when the user names it.
    name: String,
    pub(super) list: ListState,
    pub(super) editor: Option<LineEditor>,
}

pub(super) enum FormTarget {
    Record {
        collection: String,
        record: String,
    },
    Add {
        collection: String,
    },
    /// The settings of `section` in the commit group `group`.
    Group {
        section: String,
        group: String,
    },
}

/// What a row of the form is.
#[derive(Clone, Copy)]
enum Line {
    Name,
    Field(usize),
    /// A record's action, by its place in the collection's actions.
    Action(usize),
    /// A page that is an action of the record ([`registry::record_pages`]).
    Page(&'static PageOverride),
    /// The draft's Add or Save.
    Submit,
}

impl FormPage {
    pub(super) fn record(collection: &SettingsCollection, record: &SettingsRecord) -> Self {
        Self::new(FormTarget::Record {
            collection: collection.id.clone(),
            record: record.id.clone(),
        })
    }

    pub(super) fn add(collection: &SettingsCollection) -> Self {
        Self::new(FormTarget::Add {
            collection: collection.id.clone(),
        })
    }

    pub(super) fn group(section: &SettingsSection, group: &str) -> Self {
        Self::new(FormTarget::Group {
            section: section.id.clone(),
            group: group.to_string(),
        })
    }

    fn new(target: FormTarget) -> Self {
        Self {
            target,
            drafts: Map::new(),
            name: String::new(),
            list: ListState::default(),
            editor: None,
        }
    }

    fn collection<'a>(&self, catalog: &'a SettingsCatalog) -> Option<&'a SettingsCollection> {
        let id = match &self.target {
            FormTarget::Record { collection, .. } | FormTarget::Add { collection } => collection,
            FormTarget::Group { .. } => return None,
        };
        catalog
            .collections
            .iter()
            .find(|candidate| candidate.id == *id)
    }

    fn stored<'a>(&self, catalog: &'a SettingsCatalog) -> Option<&'a SettingsRecord> {
        let FormTarget::Record { record, .. } = &self.target else {
            return None;
        };
        self.collection(catalog)?
            .records
            .iter()
            .find(|candidate| candidate.id == *record)
    }

    fn section<'a>(&self, catalog: &'a SettingsCatalog) -> Option<&'a SettingsSection> {
        let FormTarget::Group { section, .. } = &self.target else {
            return None;
        };
        catalog
            .sections
            .iter()
            .find(|candidate| candidate.id == *section)
    }

    /// Whether what it edits is still in the catalog.
    pub(super) fn exists(&self, catalog: &SettingsCatalog) -> bool {
        match &self.target {
            FormTarget::Record { .. } => self.stored(catalog).is_some(),
            FormTarget::Add { .. } => self.collection(catalog).is_some(),
            FormTarget::Group { .. } => self.section(catalog).is_some(),
        }
    }

    /// Its fields' specs.
    fn fields<'a>(&self, catalog: &'a SettingsCatalog) -> Vec<&'a SettingSpec> {
        match &self.target {
            FormTarget::Group { group, .. } => self
                .section(catalog)
                .map(|section| {
                    section
                        .settings
                        .iter()
                        .filter(|spec| spec.commit_group.as_deref() == Some(group))
                        .collect()
                })
                .unwrap_or_default(),
            _ => self
                .collection(catalog)
                .map(|collection| collection.record.iter().collect())
                .unwrap_or_default(),
        }
    }

    /// The value of the field `key`: unsaved, else as it was read; a
    /// record being added starts from what its add writes (the defaults).
    pub(super) fn value(&self, catalog: &SettingsCatalog, key: &str) -> Option<SettingValue> {
        let spec = || {
            self.fields(catalog)
                .into_iter()
                .find(|spec| spec.key == key)
        };
        if let Some(draft) = self.drafts.get(key) {
            return spec()?.read(draft).ok();
        }
        match &self.target {
            FormTarget::Record { .. } => self.stored(catalog)?.values.get(key).cloned(),
            FormTarget::Add { .. } => spec()?.default.clone(),
            FormTarget::Group { .. } => self
                .section(catalog)?
                .values
                .iter()
                .find(|state| state.key == key)?
                .value
                .clone(),
        }
    }

    /// Whether the fields can be changed: a record of a collection that
    /// saves none, or one the owner writes elsewhere, is shown only.
    fn editable(&self, catalog: &SettingsCatalog) -> bool {
        match &self.target {
            FormTarget::Record { .. } => {
                self.collection(catalog).is_some_and(collection::writable)
                    && self
                        .stored(catalog)
                        .is_some_and(|record| record.read_only.is_none())
            }
            FormTarget::Add { .. } | FormTarget::Group { .. } => true,
        }
    }

    fn lines(&self, catalog: &SettingsCatalog) -> Vec<Line> {
        let mut lines = Vec::new();
        let collection = self.collection(catalog);
        if let FormTarget::Add { .. } = self.target {
            if add_action(collection).is_some_and(|action| action.names_record.is_some()) {
                lines.push(Line::Name);
            }
        }
        lines.extend(
            self.fields(catalog)
                .into_iter()
                .enumerate()
                .filter(|(_, spec)| {
                    values::holds(spec.visible_if.as_ref(), |key| self.value(catalog, key))
                })
                .map(|(index, _)| Line::Field(index)),
        );
        match &self.target {
            FormTarget::Record { collection: id, .. } => {
                lines.extend(
                    collection
                        .into_iter()
                        .flat_map(|collection| collection.actions.iter().enumerate())
                        .filter(|(_, action)| {
                            action.scope == hanafubuki_agent::SettingsActionScope::Record
                                && action.id != SAVE
                        })
                        .map(|(index, _)| Line::Action(index)),
                );
                lines.extend(registry::record_pages(id).map(Line::Page));
            }
            FormTarget::Add { .. } | FormTarget::Group { .. } => lines.push(Line::Submit),
        }
        lines
    }

    /// The field `key` takes `value`, unsaved for now.
    pub(super) fn set_draft(&mut self, key: String, value: Value) {
        self.drafts.insert(key, value);
    }

    /// The save of what is unsaved, for a record's form; `None` for a
    /// draft, saved together by its Add or Save.
    pub(super) fn live_save(&self, catalog: &SettingsCatalog) -> Option<SettingsActionRequest> {
        let FormTarget::Record { record, .. } = &self.target else {
            return None;
        };
        let collection = self.collection(catalog)?;
        let mut request = request(collection, Some(record), SAVE);
        request.values = self.drafts.clone();
        Some(request)
    }

    pub(super) fn saved(&mut self) {
        if let FormTarget::Record { .. } = self.target {
            self.drafts.clear();
        }
        self.editor = None;
    }

    pub(super) fn refused(&mut self, error: &str) {
        if let Some(editor) = self.editor.as_mut() {
            editor.refused(error);
        }
    }

    pub(super) fn paste(&mut self, text: &str) {
        if let Some(editor) = self.editor.as_mut() {
            editor.paste(text);
        }
    }

    /// `at` is the page's place in the stack, which its choices and lists
    /// change.
    pub(super) fn handle_key(
        &mut self,
        key: KeyEvent,
        catalog: &SettingsCatalog,
        at: usize,
    ) -> Effect {
        let lines = self.lines(catalog);
        let selected = lines.get(self.list.selected).copied();
        let fields = self.fields(catalog);
        if let Some(editor) = self.editor.as_mut() {
            return match (editor.handle_key(key), selected) {
                (Some(true), Some(Line::Name)) => {
                    self.name = editor.field().text().trim().to_string();
                    self.editor = None;
                    Effect::None
                }
                (Some(true), Some(Line::Field(index))) => editor.submit(fields[index], Some(at)),
                (Some(false), _) => {
                    self.editor = None;
                    Effect::None
                }
                _ => Effect::None,
            };
        }
        match key.code {
            KeyCode::Esc => Effect::Pop,
            KeyCode::Enter => self.activate(catalog, at),
            KeyCode::Char(' ') => match selected {
                Some(Line::Field(index)) if fields[index].kind == SettingKind::Boolean => {
                    self.activate(catalog, at)
                }
                _ => Effect::None,
            },
            KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => match selected {
                Some(Line::Field(index)) => self.reset(fields[index], at, catalog),
                _ => Effect::None,
            },
            _ => {
                self.list.navigate(key, &lines, |_| true);
                Effect::None
            }
        }
    }

    /// Enter on the selected row.
    pub(super) fn activate(&mut self, catalog: &SettingsCatalog, at: usize) -> Effect {
        let lines = self.lines(catalog);
        let collection = self.collection(catalog);
        match lines.get(self.list.selected).copied() {
            Some(Line::Name) => {
                let name = SettingValue::Text(self.name.clone());
                let spec = SettingSpec::new(
                    "",
                    "",
                    SettingKind::Text {
                        maximum_chars: None,
                        secret: false,
                        nullable: false,
                    },
                );
                self.editor = Some(LineEditor::new(&spec, Some(&name)));
                Effect::None
            }
            Some(Line::Field(index)) if self.editable(catalog) => {
                let spec = self.fields(catalog)[index];
                let value = self.value(catalog, &spec.key);
                match &spec.kind {
                    SettingKind::Boolean => Effect::Edit {
                        key: spec.key.clone(),
                        value: Value::Bool(!matches!(value, Some(SettingValue::Boolean(true)))),
                        at: Some(at),
                        pop: false,
                    },
                    SettingKind::Choice { .. } => Effect::Push(Box::new(Page::Choice(
                        ChoicePage::edit(spec, value.as_ref(), at),
                    ))),
                    SettingKind::TextList { .. } => {
                        Effect::Push(Box::new(Page::List(ListPage::new(spec.clone(), at))))
                    }
                    _ => {
                        self.editor = Some(LineEditor::new(spec, value.as_ref()));
                        Effect::None
                    }
                }
            }
            Some(Line::Action(index)) => {
                let (Some(collection), FormTarget::Record { record, .. }) =
                    (collection, &self.target)
                else {
                    return Effect::None;
                };
                let action = &collection.actions[index];
                let request = request(collection, Some(record), &action.id);
                if action.confirm {
                    let subject = self
                        .stored(catalog)
                        .map_or("", |record| record.label.as_str());
                    return Effect::Push(Box::new(Page::Choice(ChoicePage::confirm(
                        &action.label,
                        subject,
                        request,
                    ))));
                }
                Effect::Run {
                    request,
                    pop: false,
                }
            }
            Some(Line::Page(page)) => match &self.target {
                FormTarget::Record { record, .. } => Effect::Open(page, Some(record.clone())),
                _ => Effect::None,
            },
            Some(Line::Submit) => self.submit(collection),
            Some(Line::Field(_)) | None => Effect::None,
        }
    }

    /// The draft saved: added, or written together.
    fn submit(&self, collection: Option<&SettingsCollection>) -> Effect {
        match (&self.target, collection) {
            (FormTarget::Add { .. }, Some(collection)) => {
                let named = add_action(Some(collection))
                    .is_some_and(|action| action.names_record.is_some());
                let mut request = request(collection, named.then_some(self.name.as_str()), ADD);
                request.values = self.drafts.clone();
                Effect::Run {
                    request,
                    pop: false,
                }
            }
            (FormTarget::Group { .. }, _) if !self.drafts.is_empty() => {
                Effect::Write(SettingsWrite {
                    changes: self
                        .drafts
                        .iter()
                        .map(|(key, value)| SettingChange {
                            key: key.clone(),
                            op: SettingWriteOp::Set(value.clone()),
                        })
                        .collect(),
                })
            }
            _ => Effect::Pop,
        }
    }

    /// `r`: a draft forgotten; a record's field unset (a profile's key
    /// removed, a filter back to its owner's), which the owner refuses for
    /// a field that is always set.
    fn reset(&mut self, spec: &SettingSpec, at: usize, catalog: &SettingsCatalog) -> Effect {
        if self.drafts.remove(&spec.key).is_some() {
            return Effect::None;
        }
        match &self.target {
            FormTarget::Record { .. } if self.editable(catalog) => Effect::Edit {
                key: spec.key.clone(),
                value: Value::Null,
                at: Some(at),
                pop: false,
            },
            _ => Effect::None,
        }
    }

    pub(super) fn title(&self, catalog: &SettingsCatalog) -> String {
        match &self.target {
            FormTarget::Record { .. } => match (self.collection(catalog), self.stored(catalog)) {
                (Some(collection), Some(record)) if collection::singleton(collection).is_some() => {
                    record.label.clone()
                }
                (Some(collection), Some(record)) => {
                    format!("{} › {}", collection.title, record.label)
                }
                _ => String::new(),
            },
            FormTarget::Add { .. } => {
                let collection = self.collection(catalog);
                match (collection, add_action(collection)) {
                    (Some(collection), Some(action)) => {
                        format!("{} › {}", collection.title, action.label)
                    }
                    _ => String::new(),
                }
            }
            FormTarget::Group { .. } => self
                .section(catalog)
                .map(|section| section.title.clone())
                .unwrap_or_default(),
        }
    }

    pub(super) fn subtitle(&self, catalog: &SettingsCatalog) -> String {
        match &self.target {
            FormTarget::Record { .. } if !self.editable(catalog) => self
                .stored(catalog)
                .and_then(|record| record.read_only.clone())
                .unwrap_or_else(|| "Shown only; it is not written here.".to_string()),
            FormTarget::Record { .. } => match self.stored(catalog) {
                Some(SettingsRecord {
                    issue: Some(issue), ..
                }) => format!("⚠ {issue}"),
                Some(record) => record.summary.clone(),
                None => String::new(),
            },
            FormTarget::Add { .. } => "Its fields are saved together when it is added.".to_string(),
            FormTarget::Group { .. } => "These settings are saved together.".to_string(),
        }
    }

    pub(super) fn view<'a>(&'a self, catalog: &SettingsCatalog) -> Vec<Row<'a>> {
        let fields = self.fields(catalog);
        let editable = self.editable(catalog);
        let record = self.stored(catalog);
        let collection = self.collection(catalog);
        let selected = self.list.selected;
        self.lines(catalog)
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                let editing = (index == selected)
                    .then_some(self.editor.as_ref())
                    .flatten();
                let mut row = match line {
                    Line::Name => {
                        let label = add_action(collection)
                            .and_then(|action| action.names_record.as_deref())
                            .unwrap_or("Name");
                        let value = if self.name.is_empty() {
                            "(required)"
                        } else {
                            &self.name
                        };
                        let mut row = Row::open(label, value, "");
                        row.hint = "enter edit".to_string();
                        row
                    }
                    Line::Field(field) => {
                        let spec = fields[field];
                        let drafted = self.drafts.contains_key(&spec.key);
                        let secret_set = !drafted
                            && record.is_some_and(|record| record.secrets_set.contains(&spec.key));
                        let value = self.value(catalog, &spec.key);
                        let mut row = Row::setting(spec, value.as_ref(), secret_set, false);
                        if drafted && matches!(self.target, FormTarget::Record { .. }) {
                            row.source = Some("unsaved".to_string());
                        }
                        if !editable {
                            row.hint = "read-only".to_string();
                            row.style = RowStyle::Disabled;
                        }
                        row
                    }
                    Line::Action(action) => {
                        let label = collection
                            .map_or("", |collection| collection.actions[action].label.as_str());
                        let mut row = Row::open(label, "", "");
                        row.hint = "enter run".to_string();
                        row
                    }
                    Line::Page(page) => {
                        let label = match page.attach {
                            Attach::Record { label, .. } => label,
                            _ => page.id,
                        };
                        let mut row = Row::open(label, "›", "");
                        row.hint = "enter open".to_string();
                        row
                    }
                    Line::Submit => {
                        let label = match &self.target {
                            FormTarget::Add { .. } => {
                                add_action(collection).map_or("Add", |action| action.label.as_str())
                            }
                            _ => "Save",
                        };
                        let mut row = Row::open(label, "", "");
                        row.hint = "enter save".to_string();
                        row
                    }
                };
                row.editor = editing;
                row
            })
            .collect()
    }
}

fn add_action(
    collection: Option<&SettingsCollection>,
) -> Option<&hanafubuki_agent::SettingsAction> {
    collection?.actions.iter().find(|action| action.id == ADD)
}
