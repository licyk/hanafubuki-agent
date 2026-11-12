// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The plugin browser, one view through its pages (Codex's
//! `chatwidget/plugins.rs`): the tabs, switched with ←/→; a plugin's
//! details and its actions; the confirmations; the typed sources. A call
//! leaves the page showing what runs ("Installing…") until everything is
//! read again, then shows where the call leads: an install with hooks asks
//! to trust them, an uninstall goes back to the installed plugins.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::options;
use super::pages::{self, After, Choice, OptionsOf, Page, PageView, Prompt, Tab};
use crate::agent_bridge::{PluginsCall, PluginsDone, PluginsReply, PluginsState};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, FormEvent, FormItem, FormView, ListSelectionView,
};
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;
use hanafubuki_agent::AgentPackageStatus;

const SOURCE: &str = "source";
const SUBMIT: &str = "submit";

pub(super) struct PluginsBrowser {
    tx: AppEventSender,
    /// `None` until everything is read.
    state: Option<PluginsState>,
    page: Page,
    /// The pages to go back to, with the row each had highlighted.
    back: Vec<(Page, Option<usize>)>,
    list: Option<ListSelectionView>,
    choices: Vec<Option<Choice>>,
    /// The typed source of a `Prompt` page.
    form: Option<FormView>,
    /// Under the list: what runs, what a call did, or why it failed.
    notice: Option<Line<'static>>,
    /// Where the call that runs leads once answered.
    pending: Option<After>,
    done: bool,
}

impl PluginsBrowser {
    pub(super) fn new(tx: AppEventSender) -> Self {
        Self {
            tx,
            state: None,
            page: Page::Tab(Tab::Installed),
            back: Vec::new(),
            list: None,
            choices: Vec::new(),
            form: None,
            notice: None,
            pending: None,
            done: false,
        }
    }

    fn send(&self, call: PluginsCall) {
        self.tx
            .send(AppEvent::Op(Op::Command(AgentCommand::Plugins(call))));
    }

    fn selected(&self) -> Option<usize> {
        self.list
            .as_ref()
            .and_then(ListSelectionView::selected_index)
    }

    /// Opens `page`, remembering the one shown to go back to.
    fn open(&mut self, page: Page) {
        let selected = self.selected();
        self.back.push((self.page.clone(), selected));
        self.show(page, Some(0));
    }

    fn go_back(&mut self) {
        match self.back.pop() {
            Some((page, selected)) => self.show(page, selected),
            None => self.done = true,
        }
    }

    /// Shows `page` with its row `selected` highlighted; a page whose
    /// subject is gone shows its tab instead.
    fn show(&mut self, page: Page, selected: Option<usize>) {
        self.page = page;
        self.form = None;
        let Some(state) = self.state.as_ref() else {
            self.list = None;
            return;
        };
        if let Page::Prompt(prompt) = self.page {
            let settings = self
                .settings_package()
                .map(super::code::settings_text)
                .unwrap_or_default();
            self.form = Some(prompt_form(prompt, &settings));
            self.list = None;
            return;
        }
        if let Page::Options(target) = &self.page {
            if let Some(form) = options::form(state, target) {
                self.form = Some(form);
                self.list = None;
                return;
            }
        }
        let Some(view) = page_view(state, &self.page) else {
            let tab = self.tab();
            self.back.clear();
            return self.show(Page::Tab(tab), None);
        };
        let params = crate::bottom_pane::SelectionViewParams {
            footer_note: self.notice.clone(),
            initial_selected_idx: selected,
            ..view.params
        };
        self.choices = view.choices;
        self.list = Some(ListSelectionView::new(params, self.tx.clone()));
    }

    /// The same page again, over the state as it is now.
    fn refresh(&mut self) {
        let selected = self.selected();
        self.show(self.page.clone(), selected);
    }

    /// The tab the page shown belongs to.
    fn tab(&self) -> Tab {
        std::iter::once(&self.page)
            .chain(self.back.iter().map(|(page, _)| page))
            .find_map(|page| match page {
                Page::Tab(tab) => Some(*tab),
                _ => None,
            })
            .unwrap_or(Tab::Installed)
    }

    fn choose(&mut self, choice: Choice) {
        match choice {
            Choice::Open(page) => self.open(page),
            Choice::Back => self.go_back(),
            Choice::Call { call, busy, after } => self.call(call, &busy, after),
        }
    }

    fn call(&mut self, call: PluginsCall, busy: &str, after: After) {
        self.send(call);
        self.pending = Some(after);
        self.notice = Some(Line::from(busy.to_string().dim()));
        self.refresh();
    }

    fn selected_package(&self) -> Option<&AgentPackageStatus> {
        self.state.as_ref()?.packages.get(self.selected()?)
    }

    /// The keys a tab takes besides the list's: ←/→, and each tab's own.
    fn handle_tab_key(&mut self, tab: Tab, key: KeyEvent) -> bool {
        let plain = key.modifiers == KeyModifiers::NONE;
        let control = key.modifiers == KeyModifiers::CONTROL;
        match (tab, key.code) {
            (_, KeyCode::Left) => self.show(Page::Tab(tab.step(-1)), Some(0)),
            (_, KeyCode::Right) => self.show(Page::Tab(tab.step(1)), Some(0)),
            (Tab::Installed, KeyCode::Char(' ')) if plain => {
                let Some(package) = self.selected_package() else {
                    return true;
                };
                let call = PluginsCall::SetEnabled {
                    id: package.id.clone(),
                    enabled: !package.enabled,
                };
                self.call(call, "Saving…", After::Stay);
            }
            (Tab::Marketplaces, KeyCode::Char('u')) if control => {
                if let Some(marketplace) = self.selected_marketplace() {
                    let call = PluginsCall::UpgradeMarketplace(marketplace);
                    self.call(call, "Upgrading…", After::Stay);
                }
            }
            (Tab::Marketplaces, KeyCode::Char('r')) if control => {
                if let Some(marketplace) = self.selected_marketplace() {
                    self.open(Page::ConfirmRemoveMarketplace(marketplace));
                }
            }
            (Tab::BuiltIn, KeyCode::Char(' ')) if plain => {
                if let Some(Some(choice)) = self.selected().and_then(|i| self.choices.get(i)) {
                    self.choose(choice.clone());
                }
            }
            (Tab::Installed, KeyCode::Char('o')) if plain => {
                if let Some(target) = self.selected_package().and_then(options::of_package) {
                    self.open(Page::Options(target));
                }
            }
            (Tab::BuiltIn, KeyCode::Char('o')) if plain => {
                let target = self.selected().and_then(|index| {
                    options::of_builtin(self.state.as_ref()?.builtins.get(index)?)
                });
                if let Some(target) = target {
                    self.open(Page::Options(target));
                }
            }
            (Tab::BuiltIn, KeyCode::Char('r')) if plain => {
                let name = self.selected().and_then(|index| {
                    let state = self.state.as_ref()?;
                    Some(state.builtins.get(index)?.name.clone())
                });
                if let Some(name) = name {
                    self.call(PluginsCall::RetryBuiltin(name), "Starting it…", After::Stay);
                }
            }
            _ => return false,
        }
        true
    }

    /// The plugin whose page opened the settings form.
    fn settings_package(&self) -> Option<&AgentPackageStatus> {
        let Some((Page::Package(id), _)) = self.back.last() else {
            return None;
        };
        let state = self.state.as_ref()?;
        state.packages.iter().find(|package| package.id == *id)
    }

    fn selected_marketplace(&self) -> Option<String> {
        let state = self.state.as_ref()?;
        Some(state.marketplaces.get(self.selected()?)?.name.clone())
    }

    fn handle_form_key(&mut self, prompt: Prompt, key: KeyEvent) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        match form.handle_key(key) {
            FormEvent::Cancel => self.go_back(),
            FormEvent::Action(_) => {
                let source = form.text(SOURCE).trim().to_string();
                if prompt == Prompt::Settings {
                    return self.submit_settings(&source);
                }
                if source.is_empty() {
                    form.set_error(SOURCE, "Type a source.");
                    return;
                }
                let (call, busy) = match prompt {
                    Prompt::AddMarketplace => (PluginsCall::AddMarketplace(source), "Adding…"),
                    Prompt::InstallDirectory => (PluginsCall::Install(source), "Installing…"),
                    Prompt::Settings => return,
                };
                form.set_status(vec![busy.dim().into()]);
                self.send(call);
                self.pending = Some(After::Stay);
            }
            FormEvent::None => {}
        }
    }

    fn handle_options_key(&mut self, target: OptionsOf, key: KeyEvent) {
        let (Some(form), Some(state)) = (self.form.as_mut(), self.state.as_ref()) else {
            return;
        };
        match form.handle_key(key) {
            FormEvent::Cancel => self.go_back(),
            FormEvent::Action(_) => {
                if let Some(call) = options::save(form, state, &target) {
                    form.set_status(vec!["Saving…".dim().into()]);
                    self.send(call);
                    self.pending = Some(After::Stay);
                }
            }
            FormEvent::None => {}
        }
    }

    /// Saves `text`, the settings as JSON: an object, or nothing to remove
    /// them.
    fn submit_settings(&mut self, text: &str) {
        let id = self.settings_package().map(|package| package.id.clone());
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let Some(id) = id else {
            return form.set_error(SOURCE, "The plugin is gone.");
        };
        let settings = match text {
            "" => serde_json::Value::Null,
            text => match serde_json::from_str::<serde_json::Value>(text) {
                Ok(settings @ serde_json::Value::Object(_)) => settings,
                _ => return form.set_error(SOURCE, "Type a JSON object, such as {\"key\": 1}."),
            },
        };
        form.set_status(vec!["Saving…".dim().into()]);
        self.send(PluginsCall::SetSettings { id, settings });
        self.pending = Some(After::Stay);
    }

    fn reply(&mut self, reply: PluginsReply) {
        let after = self.pending.take();
        match reply {
            PluginsReply::Loaded { state, done } => {
                self.state = Some(*state);
                self.notice = None;
                match done {
                    Some(PluginsDone::Notice(text)) => {
                        self.notice = Some(Line::from(text.green()));
                    }
                    Some(PluginsDone::Installed { package, hooks }) => {
                        let id = package.id.clone();
                        self.notice = Some(Line::from(
                            format!("{id} is installed (version {}).", package.version).green(),
                        ));
                        self.back = vec![(Page::Tab(Tab::Installed), None)];
                        let page = if hooks.is_empty() {
                            Page::Package(id)
                        } else {
                            Page::TrustHooks { id, hooks }
                        };
                        return self.show(page, Some(0));
                    }
                    None => {}
                }
                match after {
                    Some(After::Tab(tab)) => {
                        self.back.clear();
                        self.show(Page::Tab(tab), None);
                    }
                    Some(After::Stay) => match &self.page {
                        // A typed source or options that worked lead back.
                        Page::Prompt(_) | Page::Options(_) => self.go_back(),
                        Page::TrustHooks { id, .. } => {
                            let page = Page::Package(id.clone());
                            self.show(page, Some(0));
                        }
                        _ => self.refresh(),
                    },
                    None => self.refresh(),
                }
            }
            PluginsReply::Failed { error, state } => {
                if let Some(state) = state {
                    self.state = Some(*state);
                }
                if self.state.is_none() {
                    self.state = Some(PluginsState::default());
                }
                if let (Some(form), Page::Prompt(_) | Page::Options(_)) =
                    (self.form.as_mut(), &self.page)
                {
                    form.set_status(vec![error.red().into()]);
                    return;
                }
                self.notice = Some(Line::from(error.red()));
                self.refresh();
            }
        }
    }
}

impl BottomPaneView for PluginsBrowser {
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if let Page::Prompt(prompt) = self.page {
            return self.handle_form_key(prompt, key);
        }
        if let (Page::Options(target), Some(_)) = (&self.page, &self.form) {
            return self.handle_options_key(target.clone(), key);
        }
        if self.list.is_none() {
            self.done = key.code == KeyCode::Esc;
            return;
        }
        if key.code == KeyCode::Esc {
            return self.go_back();
        }
        if let Page::Tab(tab) = self.page {
            if self.handle_tab_key(tab, key) {
                return;
            }
        }
        let Some(list) = self.list.as_mut() else {
            return;
        };
        list.handle_key_event(key);
        let chosen = list
            .take_last_selected_index()
            .and_then(|index| self.choices.get(index).cloned().flatten());
        if let Some(choice) = chosen {
            self.choose(choice);
        }
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        self.form
            .as_mut()
            .is_some_and(|form| form.handle_paste(&pasted))
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match reply {
            CommandReply::Plugins(reply) => self.reply(reply),
            reply => return Some(reply),
        }
        None
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.done = true;
        CancellationEvent::Handled
    }
}

impl Renderable for PluginsBrowser {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if let Some(form) = &self.form {
            return form.render(area, buf);
        }
        match &self.list {
            Some(list) => list.render(area, buf),
            None => Paragraph::new(vec![
                Line::from("Plugins".bold()),
                Line::from("Reading the plugins…".dim()),
            ])
            .render(area, buf),
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        if let Some(form) = &self.form {
            return form.desired_height(width);
        }
        self.list
            .as_ref()
            .map_or(2, |list| list.desired_height(width))
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.form.as_ref().and_then(|form| form.cursor_pos(area))
    }
}

/// The list of `page` over `state`; `None` when its subject is gone.
fn page_view(state: &PluginsState, page: &Page) -> Option<PageView> {
    let package = |id: &str| state.packages.iter().find(|package| package.id == id);
    let marketplace = |name: &str| {
        state
            .marketplaces
            .iter()
            .find(|marketplace| marketplace.name == name)
    };
    Some(match page {
        Page::Tab(tab) => pages::tab_page(state, *tab),
        Page::Package(id) => pages::package_page(package(id)?),
        Page::Marketplace(name) => pages::marketplace_page(marketplace(name)?),
        Page::Offered {
            marketplace: name,
            plugin,
        } => {
            let marketplace = marketplace(name)?;
            let offered = marketplace
                .plugins
                .iter()
                .find(|offered| offered.name == *plugin)?;
            if offered.installed {
                pages::package_page(package(&offered.id)?)
            } else {
                pages::offered_page(marketplace, offered)
            }
        }
        Page::ConfirmUninstall(id) => pages::confirm_uninstall(&package(id)?.id),
        Page::ConfirmRemoveMarketplace(name) => {
            pages::confirm_remove_marketplace(&marketplace(name)?.name)
        }
        Page::TrustHooks { id, hooks } => pages::trust_hooks_page(id, hooks),
        Page::Prompt(_) | Page::Options(_) => return None,
    })
}

/// The form of `prompt`; a settings form starts from `settings`.
fn prompt_form(prompt: Prompt, settings: &str) -> FormView {
    let (title, subtitle, label, placeholder, submit) = match prompt {
        Prompt::AddMarketplace => (
            "Add a marketplace",
            "Its plugins are then offered on the Marketplaces tab.",
            "Source",
            "owner/repo, a git URL, or a local marketplace path",
            "Add marketplace",
        ),
        Prompt::InstallDirectory => (
            "Install a plugin from a directory",
            "Its manifest names it; it is filed under the marketplace `local`.",
            "Directory",
            "the plugin's directory",
            "Install plugin",
        ),
        Prompt::Settings => (
            "Plugin settings",
            "A JSON object, checked against what its manifest says; empty removes them. Its code starts again with them.",
            "Settings",
            "{\"key\": \"value\"}",
            "Save settings",
        ),
    };
    let value = if prompt == Prompt::Settings {
        settings
    } else {
        ""
    };
    FormView::new(
        title,
        vec![
            FormItem::text(SOURCE, label, value).placeholder(placeholder),
            FormItem::action(SUBMIT, submit),
        ],
    )
    .with_subtitle(subtitle)
}
