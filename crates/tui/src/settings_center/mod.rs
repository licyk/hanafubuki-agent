// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/config`'s settings center (Codex's `config_panel/`, full screen): the
//! facade's settings catalog as tabs of label · value · description rows,
//! with no code of its own per setting.
//!
//! The center is a stack of pages over the catalog: the tabs (`main`), a
//! collection's records (`collection`), a record or a commit group as a
//! form (`form`), a choice, a list of texts or a confirmation (`editor`),
//! and a command's views shown as a page (`registry`). A page's keys turn
//! into an [`Effect`]; what it asks of the Agent is a [`CenterRequest`]
//! the app runs off its loop, one at a time, and the answer
//! ([`CatalogReply`]) brings the catalog read again after it. Nothing here
//! waits for the Agent.

mod collection;
mod editor;
mod form;
mod main_page;
mod registry;
mod render;
#[cfg(test)]
mod tests;
mod values;

use hanafubuki_agent::{
    SettingApply, SettingChange, SettingValue, SettingWriteOp, SettingsActionRequest,
    SettingsCatalog, SettingsWrite, SettingsWriteOutcome,
};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;
use serde_json::Value;

use crate::agent_bridge::{CatalogCall, CatalogOutcome, CatalogReply};
use collection::CollectionPage;
use editor::{ChoicePage, ListPage, ListState};
use form::{FormPage, FormTarget};
use main_page::MainPage;
pub(crate) use registry::{Attach, PageOverride};
use render::Layout;

/// The rows a turn of the wheel moves.
const WHEEL_ROWS: usize = 3;

/// What the app does for the center.
#[derive(Debug)]
pub(crate) enum CenterRequest {
    /// Runs the call off the loop; its reply comes back to
    /// [`SettingsCenter::on_reply`].
    Call(u64, CatalogCall),
    /// Shows the views of the page now on top, on its record.
    OpenPage(&'static PageOverride, Option<String>),
    Close,
}

pub(crate) struct SettingsCenter {
    catalog: Option<SettingsCatalog>,
    /// Why the catalog could not be read, while none was.
    read_error: Option<String>,
    /// The tabs at the bottom, the open page on top.
    pages: Vec<Page>,
    notice: Notice,
    /// The call the Agent is answering, and the page it was made from.
    pending: Option<(u64, Pending)>,
    next_request: u64,
    /// The latest call whose answer came.
    answered: u64,
    requests: Vec<CenterRequest>,
    /// The number of the latest check of a page that is a command's views.
    page_checks: u64,
    /// What `/config` named, opened once the catalog is read, and the
    /// record a record's page is opened on.
    target: Option<(String, Option<String>)>,
    /// Where the last frame put the tabs and the rows, for the pointer.
    layout: Layout,
}

/// One page of the stack.
enum Page {
    Main(MainPage),
    Collection(CollectionPage),
    Form(FormPage),
    Choice(ChoicePage),
    List(ListPage),
    /// A command's views ([`registry`]).
    Hosted(&'static PageOverride),
}

impl Page {
    fn list_mut(&mut self) -> Option<&mut ListState> {
        match self {
            Page::Main(page) => Some(&mut page.list),
            Page::Collection(page) => Some(&mut page.list),
            Page::Form(page) => Some(&mut page.list),
            Page::Choice(page) => Some(&mut page.list),
            Page::List(page) => Some(&mut page.list),
            Page::Hosted(_) => None,
        }
    }
}

/// What a page asks for after a key.
enum Effect {
    None,
    Push(Box<Page>),
    Pop,
    Close,
    /// A page, on a record when it is a record's.
    Open(&'static PageOverride, Option<String>),
    /// A field of the page `at` (the page itself when `None`) takes `value`;
    /// `pop` closes the page that chose it.
    Edit {
        key: String,
        value: Value,
        at: Option<usize>,
        pop: bool,
    },
    /// The setting `key` is removed from the user's file.
    Reset(String),
    /// Writes settings together (a commit group).
    Write(SettingsWrite),
    /// Runs an action of a collection; `pop` closes the confirmation.
    Run {
        request: SettingsActionRequest,
        pop: bool,
    },
}

/// What a call answers for.
enum Pending {
    Read,
    /// A change of a field of the page at this index.
    Edit(usize),
    /// An action run from the page at this index.
    Action(usize, String),
}

/// The line under the title.
enum Notice {
    Ready,
    Busy,
    Done(String),
    Failed(String),
}

impl SettingsCenter {
    /// The center reading the catalog, opening `target` (a tab, a section,
    /// a collection or a page; a record's page on `record`) once it is read.
    pub(crate) fn new(target: Option<String>, record: Option<String>) -> Self {
        let mut center = Self {
            catalog: None,
            read_error: None,
            pages: vec![Page::Main(MainPage::default())],
            notice: Notice::Ready,
            pending: None,
            next_request: 0,
            answered: 0,
            requests: Vec::new(),
            page_checks: 0,
            target: target.map(|target| (target, record)),
            layout: Layout::default(),
        };
        center.call(Pending::Read, CatalogCall::Read);
        center
    }

    /// What the app is to do, since it last asked.
    pub(crate) fn take_requests(&mut self) -> Vec<CenterRequest> {
        std::mem::take(&mut self.requests)
    }

    /// The command whose views the page on top is.
    pub(crate) fn hosted_command(&self) -> Option<&'static str> {
        match self.pages.last() {
            Some(Page::Hosted(page)) => Some(page.command.name),
            _ => None,
        }
    }

    /// The number of a new check of the page that is a command's views.
    pub(crate) fn next_page_check(&mut self) -> u64 {
        self.page_checks += 1;
        self.page_checks
    }

    /// Whether `check` is the latest: an earlier one was sent before a
    /// key whose step has not shown its view yet.
    pub(crate) fn is_latest_page_check(&self, check: u64) -> bool {
        check == self.page_checks
    }

    /// What a page that is a command's views saved itself (the keymap,
    /// the presets, the theme), said under the title as the center's own
    /// writes are: the transcript it also goes into is hidden while the
    /// center is open.
    pub(crate) fn report(&mut self, outcome: Result<String, String>) {
        self.notice = match outcome {
            Ok(message) => Notice::Done(message),
            Err(error) => Notice::Failed(error),
        };
    }

    /// The command's views closed: back to the row that opened them, and
    /// the values they may have saved read again.
    pub(crate) fn hosted_closed(&mut self) {
        if matches!(self.pages.last(), Some(Page::Hosted(_))) {
            self.pages.pop();
            if self.pending.is_none() {
                self.call(Pending::Read, CatalogCall::Read);
            }
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.requests.push(CenterRequest::Close);
            return;
        }
        let Some(catalog) = self.catalog.as_ref() else {
            if key.code == KeyCode::Esc {
                self.requests.push(CenterRequest::Close);
            }
            return;
        };
        let top = self.pages.len() - 1;
        let items = self.list_items(top);
        let effect = match &mut self.pages[top] {
            Page::Main(page) => page.handle_key(key, catalog),
            Page::Collection(page) => page.handle_key(key, catalog),
            Page::Form(page) => page.handle_key(key, catalog, top),
            Page::Choice(page) => page.handle_key(key),
            Page::List(page) => page.handle_key_with(key, &items),
            Page::Hosted(_) => Effect::None,
        };
        self.apply(effect);
    }

    /// The pointer: the wheel moves the selection, a click on a tab shows
    /// it, a click on a row selects it, and a click on the selected row
    /// does what Enter does.
    pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) {
        let step = match mouse.kind {
            MouseEventKind::ScrollUp => Some(KeyCode::Up),
            MouseEventKind::ScrollDown => Some(KeyCode::Down),
            _ => None,
        };
        if let Some(code) = step {
            if !self.editing() {
                for _ in 0..WHEEL_ROWS {
                    self.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
                }
            }
            return;
        }
        let top = self.pages.len() - 1;
        let Some(catalog) = self.catalog.as_ref() else {
            return;
        };
        // A field being edited keeps the selection.
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) || self.editing() {
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        if let Some((_, tab)) = self.layout.tabs.iter().find(|(cell, _)| cell.contains(at)) {
            if let Page::Main(page) = &mut self.pages[top] {
                page.show_tab(catalog, *tab);
            }
            return;
        }
        if !self.layout.list.contains(at) {
            return;
        }
        let Some(&(_, index)) = self.layout.rows.iter().find(|(y, _)| *y == mouse.row) else {
            return;
        };
        let Some(list) = self.pages[top].list_mut() else {
            return;
        };
        if list.selected != index {
            list.selected = index;
            return;
        }
        let items = self.list_items(top);
        let effect = match &mut self.pages[top] {
            Page::Main(page) if page.editor.is_none() => {
                let rows = page.rows(catalog);
                if !rows.get(index).is_some_and(main_page::selectable) {
                    return;
                }
                page.activate(catalog, &rows)
            }
            Page::Collection(page) => page.activate_at(catalog),
            Page::Form(page) if page.editor.is_none() => page.activate(catalog, top),
            Page::Choice(page) => page.choose(),
            Page::List(page) => {
                page.handle_key_with(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &items)
            }
            _ => Effect::None,
        };
        self.apply(effect);
    }

    /// Pasted text, into the field being edited.
    pub(crate) fn handle_paste(&mut self, text: &str) {
        match self.pages.last_mut() {
            Some(Page::Main(page)) => page.paste(text),
            Some(Page::Form(page)) => page.paste(text),
            Some(Page::List(page)) => page.paste(text),
            _ => {}
        }
    }

    /// The answer to a call: the catalog read after it, and what the call
    /// did said under the title.
    pub(crate) fn on_reply(&mut self, reply: CatalogReply) {
        // A read answered after a later call's answer is older than it.
        let newest = reply.request > self.answered;
        self.answered = self.answered.max(reply.request);
        match reply.catalog {
            Ok(catalog) if newest => {
                self.catalog = Some(catalog);
                self.read_error = None;
            }
            Ok(_) => {}
            Err(error) if self.catalog.is_none() => self.read_error = Some(error),
            Err(error) => {
                self.notice = Notice::Failed(format!("The settings were not read again: {error}"))
            }
        }
        let pending = match &self.pending {
            Some((id, _)) if *id == reply.request => {
                self.pending.take().map(|(_, pending)| pending)
            }
            _ => None,
        };
        if let Some(pending) = pending {
            match reply.outcome {
                Ok(outcome) => self.done(pending, outcome),
                Err(error) => self.failed(pending, error),
            }
        }
        self.prune();
        if self.catalog.is_some() {
            if let Some((target, record)) = self.target.take() {
                self.open_target(&target, record);
            }
        }
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::None => {}
            Effect::Push(page) => self.pages.push(*page),
            Effect::Pop => {
                if self.pages.len() > 1 {
                    self.pages.pop();
                }
            }
            Effect::Close => self.requests.push(CenterRequest::Close),
            Effect::Open(page, record) => self.open_page(page, record),
            Effect::Edit {
                key,
                value,
                at,
                pop,
            } => {
                let at = at.unwrap_or(self.pages.len() - 1);
                if pop {
                    self.pages.pop();
                }
                self.edit(at, key, value);
            }
            Effect::Reset(key) => self.write(
                0,
                SettingsWrite {
                    changes: vec![SettingChange {
                        key,
                        op: SettingWriteOp::Reset,
                    }],
                },
            ),
            Effect::Write(write) => self.write(self.pages.len() - 1, write),
            Effect::Run { request, pop } => {
                if pop {
                    self.pages.pop();
                }
                let at = self.pages.len() - 1;
                let action = request.action.clone();
                self.send(Pending::Action(at, action), CatalogCall::Action(request));
            }
        }
    }

    /// The field `key` of the page at `at` takes `value`: a setting is
    /// written at once, a record's field saved with what it holds unsaved,
    /// a form that is a draft keeps it.
    fn edit(&mut self, at: usize, key: String, value: Value) {
        let catalog = self.catalog.as_ref();
        match self.pages.get_mut(at) {
            Some(Page::Main(_)) => self.write(
                at,
                SettingsWrite {
                    changes: vec![SettingChange {
                        key,
                        op: SettingWriteOp::Set(value),
                    }],
                },
            ),
            Some(Page::Form(form)) => {
                form.set_draft(key, value);
                match catalog.and_then(|catalog| form.live_save(catalog)) {
                    Some(request) => self.send(Pending::Edit(at), CatalogCall::Action(request)),
                    None => form.editor = None,
                }
            }
            _ => {}
        }
    }

    fn write(&mut self, at: usize, write: SettingsWrite) {
        self.send(Pending::Edit(at), CatalogCall::Write(write));
    }

    /// Sends a change, unless one is still being saved.
    fn send(&mut self, pending: Pending, call: CatalogCall) {
        if matches!(
            self.pending,
            Some((_, Pending::Edit(_) | Pending::Action(..)))
        ) {
            self.notice = Notice::Failed("Wait for the last change to be saved.".to_string());
            return;
        }
        self.notice = Notice::Busy;
        self.call(pending, call);
    }

    fn call(&mut self, pending: Pending, call: CatalogCall) {
        self.next_request += 1;
        self.pending = Some((self.next_request, pending));
        self.requests
            .push(CenterRequest::Call(self.next_request, call));
    }

    fn done(&mut self, pending: Pending, outcome: CatalogOutcome) {
        match (pending, outcome) {
            (Pending::Read, _) => {}
            (Pending::Edit(at), outcome) => {
                self.notice = Notice::Done(match &outcome {
                    CatalogOutcome::Written(written) => saved_notice(written),
                    _ => "Saved".to_string(),
                });
                match self.pages.get_mut(at) {
                    Some(Page::Main(page)) => page.saved(),
                    // A commit group saved together is done.
                    Some(Page::Form(page)) if matches!(page.target, FormTarget::Group { .. }) => {
                        self.pages.truncate(at)
                    }
                    Some(Page::Form(page)) => page.saved(),
                    _ => {}
                }
            }
            (Pending::Action(at, action), CatalogOutcome::Acted(acted)) => {
                let message = acted.message.clone();
                if let Some(Page::Form(form)) = self.pages.get(at) {
                    if matches!(form.target, FormTarget::Add { .. }) {
                        self.pages.truncate(at);
                        if let (Some(Page::Collection(page)), Some(record)) =
                            (self.pages.last_mut(), acted.record.as_deref())
                        {
                            page.select_record(self.catalog.as_ref(), record);
                        }
                    }
                }
                self.notice = Notice::Done(match message {
                    Some(message) => message,
                    None => match action.as_str() {
                        "save" => "Saved".to_string(),
                        "add" => "Added".to_string(),
                        "delete" => "Deleted".to_string(),
                        _ => format!("Done: {action}"),
                    },
                });
            }
            (Pending::Action(..), _) => {}
        }
    }

    fn failed(&mut self, pending: Pending, error: String) {
        match pending {
            Pending::Read => {}
            Pending::Edit(at) => match self.pages.get_mut(at) {
                Some(Page::Main(page)) => page.refused(&error),
                Some(Page::Form(page)) => page.refused(&error),
                _ => {}
            },
            Pending::Action(..) => {}
        }
        self.notice = Notice::Failed(format!("Could not save: {error}"));
    }

    /// The pages whose record or collection the catalog no longer has (a
    /// record deleted) are closed.
    fn prune(&mut self) {
        let Some(catalog) = self.catalog.as_ref() else {
            return;
        };
        let gone = self.pages.iter().position(|page| match page {
            Page::Collection(page) => page.collection(catalog).is_none(),
            Page::Form(page) => !page.exists(catalog),
            _ => false,
        });
        if let Some(gone) = gone {
            self.pages.truncate(gone.max(1));
        }
    }

    fn open_page(&mut self, page: &'static PageOverride, record: Option<String>) {
        self.pages.push(Page::Hosted(page));
        self.requests.push(CenterRequest::OpenPage(page, record));
    }

    /// Runs the action of a collection a page asked for (a Provider's
    /// model chosen), at the revision the center read, as its own.
    pub(crate) fn run_action(
        &mut self,
        collection: &str,
        record: Option<&str>,
        action: &str,
        values: serde_json::Map<String, Value>,
    ) {
        let Some(found) = self.catalog.as_ref().and_then(|catalog| {
            catalog
                .collections
                .iter()
                .find(|candidate| candidate.id == collection)
        }) else {
            return;
        };
        let mut request = collection::request(found, record, action);
        request.values = values;
        let at = self.pages.len() - 1;
        self.send(
            Pending::Action(at, action.to_string()),
            CatalogCall::Action(request),
        );
    }

    /// `/config TARGET`: a tab, a section, a collection or a page; a
    /// record's page opens over its record's form.
    fn open_target(&mut self, target: &str, record: Option<String>) {
        let Some(catalog) = self.catalog.as_ref() else {
            return;
        };
        let Some(Page::Main(main)) = self.pages.first_mut() else {
            return;
        };
        let page = registry::page(target);
        let effect = match page.map(|page| &page.attach) {
            Some(Attach::Record { collection, .. }) => main.open_target(catalog, collection),
            _ => main.open_target(catalog, target),
        };
        let Some(effect) = effect else {
            self.notice = Notice::Failed(format!(
                "There is no settings page `{target}`: a tab, a section, a collection or a page names one."
            ));
            return;
        };
        self.apply(effect);
        let (Some(page), Some(record)) = (page, record) else {
            return;
        };
        let Some(Page::Collection(list)) = self.pages.last_mut() else {
            return;
        };
        let catalog = self.catalog.as_ref();
        list.select_record(catalog, &record);
        let form = catalog
            .and_then(|catalog| list.collection(catalog))
            .and_then(|collection| {
                let stored = collection
                    .records
                    .iter()
                    .find(|stored| stored.id == record)?;
                Some(FormPage::record(collection, stored))
            });
        match form {
            Some(form) => {
                self.pages.push(Page::Form(form));
                self.open_page(page, Some(record));
            }
            None => {
                self.notice = Notice::Failed(format!("There is no `{record}` to open."));
            }
        }
    }

    /// Whether a field is being edited in the page on top.
    fn editing(&self) -> bool {
        match self.pages.last() {
            Some(Page::Main(page)) => {
                page.editor.is_some() || page.search.as_ref().is_some_and(|search| search.editing)
            }
            Some(Page::Form(page)) => page.editor.is_some(),
            Some(Page::List(page)) => page.editor.is_some(),
            _ => false,
        }
    }

    /// The items of the list the page `at` edits, as its field holds them.
    fn list_items(&self, at: usize) -> Vec<String> {
        match self.pages.get(at) {
            Some(Page::List(list)) => match self.field_value(list.at, &list.spec.key) {
                Some(SettingValue::TextList(items)) => items,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// The value of the field `key` of the page at `at`, as it is shown.
    fn field_value(&self, at: usize, key: &str) -> Option<SettingValue> {
        let catalog = self.catalog.as_ref()?;
        match self.pages.get(at)? {
            Page::Main(_) => main_page::setting_value(catalog, key).cloned(),
            Page::Form(form) => form.value(catalog, key),
            _ => None,
        }
    }
}

/// What a write said: saved, and whether a layer above the user's file
/// keeps another value, or when it applies.
fn saved_notice(written: &SettingsWriteOutcome) -> String {
    let mut notice = "Saved".to_string();
    if let Some(source) = written
        .shadowed
        .iter()
        .find_map(|(_, source)| values::source_text(source))
    {
        notice.push_str(&format!(" · the {source} layer overrides it"));
    }
    let apply = written.apply.iter().map(|(_, apply)| *apply);
    if apply.clone().any(|apply| apply == SettingApply::NextLaunch) {
        notice.push_str(" · applies at next launch");
    } else if apply
        .into_iter()
        .any(|apply| apply == SettingApply::NextSession)
    {
        notice.push_str(" · applies from the next session");
    }
    notice
}
