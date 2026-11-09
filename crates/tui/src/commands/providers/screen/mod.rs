// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The Provider screen, one view through its steps (Codex's provider list,
//! editor and model choice): the list, a deletion's confirmation, the form,
//! and the model picker after a save. Each write names the revision of
//! the registry the screen last read; when it moved, the screen reads it
//! again and keeps what the form holds.

use hanafubuki_agent::{AgentProviderRegistrySnapshot, AgentProviderSettings};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

mod models;

use super::form::{self, protocol_name, TEST};
use crate::agent_bridge::{ProviderCall, ProviderReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, FormEvent, FormView, ListSelectionView, SelectionItem,
    SelectionViewParams,
};
use crate::commands::CommandReply;
use crate::history_cell;
use crate::render::renderable::Renderable;

const CONFLICT: &str =
    "The Providers changed while this screen was open and were read again; your entries are kept.";

pub(super) struct ProvidersScreen {
    tx: AppEventSender,
    /// The first-run setup: done once a model is chosen.
    setup: bool,
    registry: Option<AgentProviderRegistrySnapshot>,
    stage: Stage,
    complete: bool,
}

enum Stage {
    Loading,
    List(ListSelectionView),
    ConfirmDelete {
        list: ListSelectionView,
        provider_id: String,
    },
    Form {
        form: FormView,
        /// `None` adds a Provider.
        provider_id: Option<String>,
        busy: bool,
    },
    Models {
        list: ListSelectionView,
        provider_id: String,
        /// The model of each item but the last, which asks for one.
        models: Vec<String>,
        /// A model was chosen and is being saved.
        busy: bool,
    },
    ModelEntry {
        form: FormView,
        provider_id: String,
    },
}

impl ProvidersScreen {
    pub(super) fn new(tx: AppEventSender, setup: bool) -> Self {
        Self {
            tx,
            setup,
            registry: None,
            stage: Stage::Loading,
            complete: false,
        }
    }

    fn send(&self, call: ProviderCall) {
        self.tx
            .send(AppEvent::Op(Op::Command(AgentCommand::Providers(call))));
    }

    fn revision(&self) -> u64 {
        self.registry
            .as_ref()
            .map_or(0, |registry| registry.revision)
    }

    fn providers(&self) -> &[AgentProviderSettings] {
        self.registry
            .as_ref()
            .map_or(&[], |registry| registry.providers.as_slice())
    }

    fn provider(&self, provider_id: &str) -> Option<&AgentProviderSettings> {
        self.providers()
            .iter()
            .find(|provider| provider.id == provider_id)
    }

    /// The setup goes to what is missing; otherwise the list.
    fn after_load(&mut self) {
        let Some(registry) = self.registry.as_ref().filter(|_| self.setup) else {
            return self.show_list(None, None);
        };
        match registry.default_provider_id.clone() {
            None if registry.providers.is_empty() => self.open_form(None),
            None => self.show_list(
                Some("No Provider is enabled: enable one (e) or add one (a).".into()),
                None,
            ),
            Some(provider_id) if registry.model_policy.model.trim().is_empty() => {
                self.open_models(provider_id)
            }
            Some(_) => self.show_list(None, None),
        }
    }

    fn show_list(&mut self, notice: Option<Line<'static>>, selected: Option<usize>) {
        let default = self
            .registry
            .as_ref()
            .and_then(|registry| registry.default_provider_id.clone());
        let model = self
            .registry
            .as_ref()
            .map(|registry| registry.model_policy.model.clone())
            .unwrap_or_default();
        let mut items: Vec<SelectionItem> = self
            .providers()
            .iter()
            .map(|provider| provider_item(provider, default.as_deref() == Some(&provider.id)))
            .collect();
        items.push(SelectionItem {
            name: "Add a Provider…".to_string(),
            description: Some("a model endpoint and its token".to_string()),
            ..Default::default()
        });
        let subtitle = if model.is_empty() {
            "No model is chosen for new sessions yet.".to_string()
        } else {
            format!("New sessions run {model} on the default Provider (★).")
        };
        let list = ListSelectionView::new(
            SelectionViewParams {
                title: Some(if self.setup {
                    "Set up a model Provider".to_string()
                } else {
                    "Providers".to_string()
                }),
                subtitle: Some(subtitle),
                footer_note: notice,
                footer_hint: Some(Line::from(
                    "enter edit · a add · d delete · e enable/disable · * default · esc close",
                )),
                items,
                initial_selected_idx: selected,
                ..Default::default()
            },
            self.tx.clone(),
        );
        self.stage = Stage::List(list);
    }

    fn open_form(&mut self, provider_id: Option<String>) {
        let provider = provider_id.as_deref().and_then(|id| self.provider(id));
        let form = form::provider_form(provider, self.setup);
        self.stage = Stage::Form {
            form,
            provider_id,
            busy: false,
        };
    }

    fn handle_list_key(&mut self, key: KeyEvent) {
        let count = self.providers().len();
        let Stage::List(list) = &self.stage else {
            return;
        };
        let selected = list.selected_index().filter(|index| *index < count);
        let plain = key.kind != KeyEventKind::Release
            && (key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT);
        let provider = selected
            .and_then(|index| self.providers().get(index))
            .cloned();
        match (plain, key.code, provider) {
            (true, KeyCode::Char('a'), _) => self.open_form(None),
            (true, KeyCode::Char('d'), Some(provider)) => self.confirm_delete(&provider),
            (true, KeyCode::Char('e'), Some(provider)) => {
                self.send(ProviderCall::SetEnabled {
                    expected_revision: self.revision(),
                    provider_id: provider.id,
                    enabled: !provider.enabled,
                });
                self.show_list(Some("Saving…".dim().into()), selected);
            }
            (true, KeyCode::Char('*'), Some(provider)) => {
                self.send(ProviderCall::SetDefault {
                    expected_revision: self.revision(),
                    provider_id: provider.id,
                    model: None,
                });
                self.show_list(Some("Saving…".dim().into()), selected);
            }
            _ => {
                let Stage::List(list) = &mut self.stage else {
                    return;
                };
                list.handle_key_event(key);
                if let Some(index) = list.take_last_selected_index() {
                    let provider_id = self.providers().get(index).map(|p| p.id.clone());
                    self.open_form(provider_id);
                } else if list.is_complete() {
                    self.complete = true;
                }
            }
        }
    }

    fn confirm_delete(&mut self, provider: &AgentProviderSettings) {
        let list = ListSelectionView::new(
            SelectionViewParams {
                title: Some(format!("Delete {}?", provider.display_name)),
                subtitle: Some(
                    "Sessions that use it keep their history but cannot run until they switch."
                        .to_string(),
                ),
                items: vec![
                    SelectionItem {
                        name: "Delete".to_string(),
                        ..Default::default()
                    },
                    SelectionItem {
                        name: "Keep it".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            self.tx.clone(),
        );
        self.stage = Stage::ConfirmDelete {
            list,
            provider_id: provider.id.clone(),
        };
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        let Stage::ConfirmDelete { list, provider_id } = &mut self.stage else {
            return;
        };
        list.handle_key_event(key);
        let accepted = list.take_last_selected_index();
        let provider_id = provider_id.clone();
        let done = accepted.is_some() || list.is_complete();
        if accepted == Some(0) {
            self.send(ProviderCall::Delete {
                expected_revision: self.revision(),
                provider_id,
            });
            self.show_list(Some("Deleting…".dim().into()), None);
        } else if done {
            self.show_list(None, None);
        }
    }

    fn handle_form_key(&mut self, key: KeyEvent) {
        let expected_revision = self.revision();
        let Stage::Form {
            form,
            provider_id,
            busy,
        } = &mut self.stage
        else {
            return;
        };
        let call = match form.handle_key(key) {
            FormEvent::Cancel => return self.show_list(None, None),
            FormEvent::Action(action) if !*busy => {
                let Some(draft) = form::draft(form) else {
                    return;
                };
                *busy = true;
                if action == TEST {
                    form.set_status(vec!["Listing the models…".dim().into()]);
                    ProviderCall::Test(draft)
                } else {
                    form.set_status(vec!["Saving…".dim().into()]);
                    ProviderCall::Write {
                        expected_revision,
                        provider_id: provider_id.clone(),
                        draft,
                    }
                }
            }
            _ => return,
        };
        self.send(call);
    }

    fn reply(&mut self, reply: ProviderReply) {
        match reply {
            ProviderReply::Loaded(registry) => {
                self.registry = Some(registry);
                match &self.stage {
                    Stage::Loading => self.after_load(),
                    Stage::List(_) => self.show_list(None, None),
                    _ => {}
                }
            }
            ProviderReply::Tested(result) => {
                if let Stage::Form { form, busy, .. } = &mut self.stage {
                    *busy = false;
                    form.set_status(vec![match result {
                        Ok(models) => format!("✓ The Provider lists {} models.", models.len())
                            .green()
                            .into(),
                        Err(error) => format!("✗ {error}").red().into(),
                    }]);
                }
            }
            ProviderReply::Saved(registry) => self.saved(registry),
            ProviderReply::Changed(registry) => {
                let chosen = match &self.stage {
                    Stage::Models { provider_id, .. } | Stage::ModelEntry { provider_id, .. } => {
                        Some(provider_id.clone())
                    }
                    _ => None,
                };
                let selected = self.list_selection();
                self.registry = Some(registry);
                match chosen {
                    Some(provider_id) => self.model_chosen(&provider_id),
                    None => self.show_list(None, selected),
                }
            }
            ProviderReply::Models {
                provider_id,
                result,
            } => self.models_listed(provider_id, result),
            ProviderReply::Failed { error, reloaded } => self.failed(error, reloaded),
        }
    }

    /// A save landed: the new Provider's id is the one the registry gained.
    fn saved(&mut self, registry: AgentProviderRegistrySnapshot) {
        let Stage::Form { provider_id, .. } = &self.stage else {
            self.registry = Some(registry);
            return;
        };
        let provider_id = provider_id.clone().or_else(|| {
            registry
                .providers
                .iter()
                .find(|provider| self.provider(&provider.id).is_none())
                .map(|provider| provider.id.clone())
        });
        self.registry = Some(registry);
        match provider_id.filter(|id| self.provider(id).is_some_and(|p| p.enabled)) {
            Some(provider_id) => self.open_models(provider_id),
            None => self.show_list(
                Some(
                    "Saved. A disabled Provider runs no session; enable it to choose its model."
                        .into(),
                ),
                None,
            ),
        }
    }

    fn failed(&mut self, error: String, reloaded: Option<AgentProviderRegistrySnapshot>) {
        let conflict = reloaded.is_some();
        if let Some(registry) = reloaded {
            self.registry = Some(registry);
        }
        let message = if conflict {
            format!("{CONFLICT} Try again. ({error})")
        } else {
            error
        };
        match &mut self.stage {
            Stage::Form { form, busy, .. } => {
                *busy = false;
                form.set_status(vec![format!("✗ {message}").red().into()]);
            }
            Stage::ModelEntry { form, .. } => {
                form.set_status(vec![format!("✗ {message}").red().into()]);
            }
            _ if self.registry.is_none() => {
                self.tx.send(AppEvent::InsertHistoryCell(Box::new(
                    history_cell::new_error_event(format!("/providers: {message}")),
                )));
                self.complete = true;
            }
            _ => {
                let selected = self.list_selection();
                self.show_list(Some(format!("✗ {message}").red().into()), selected)
            }
        }
    }

    /// The row the list highlights, to keep it when the list is rebuilt.
    fn list_selection(&self) -> Option<usize> {
        match &self.stage {
            Stage::List(list) => list.selected_index(),
            _ => None,
        }
    }

    fn content(&self) -> &dyn Renderable {
        match &self.stage {
            Stage::Loading => &LOADING,
            Stage::List(list) | Stage::ConfirmDelete { list, .. } | Stage::Models { list, .. } => {
                list
            }
            Stage::Form { form, .. } | Stage::ModelEntry { form, .. } => form,
        }
    }
}

impl BottomPaneView for ProvidersScreen {
    fn handle_key_event(&mut self, key: KeyEvent) {
        match &self.stage {
            Stage::Loading if key.code == KeyCode::Esc => self.complete = true,
            Stage::Loading => {}
            Stage::List(_) => self.handle_list_key(key),
            Stage::ConfirmDelete { .. } => self.handle_confirm_key(key),
            Stage::Form { .. } => self.handle_form_key(key),
            Stage::Models { .. } => self.handle_models_key(key),
            Stage::ModelEntry { .. } => self.handle_model_entry_key(key),
        }
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        match &mut self.stage {
            Stage::Form { form, .. } | Stage::ModelEntry { form, .. } => form.handle_paste(&pasted),
            _ => false,
        }
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match reply {
            CommandReply::Providers(reply) => {
                self.reply(reply);
                None
            }
            reply => Some(reply),
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for ProvidersScreen {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.content().render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.content().desired_height(width)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.content().cursor_pos(area)
    }
}

/// What the screen shows while the Providers are read.
struct Loading;

static LOADING: Loading = Loading;

impl Renderable for Loading {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(vec![
            Line::from("Providers".bold()),
            Line::from("Reading the Providers…".dim()),
        ])
        .render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        2
    }
}

/// A Provider in the list: ★ for the default, its protocol, endpoint and
/// whether a token is set (never the token).
fn provider_item(provider: &AgentProviderSettings, default: bool) -> SelectionItem {
    let mut description = format!(
        "{} · {} · {}",
        protocol_name(provider.protocol),
        provider.base_url,
        if provider.api_token.is_some() {
            "token set"
        } else {
            "no token"
        }
    );
    if !provider.enabled {
        description.push_str(" · disabled");
    }
    SelectionItem {
        name: format!(
            "{}{}",
            if default { "★ " } else { "  " },
            provider.display_name
        ),
        description: Some(description),
        ..Default::default()
    }
}
