// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A Provider's model page: the models it lists, one of which, or an id
//! typed in, becomes the model new sessions run, with the Provider made
//! the default. The choice is a save of the model policy, which the
//! settings center runs as its own.

use hanafubuki_agent::ModelCatalogEntry;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use serde_json::{json, Map};

use crate::agent_bridge::ProviderModels;
use crate::app::{SettingsPageAction, WorkspaceEvent};
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, FormEvent, FormItem, FormView, ListSelectionView,
    SelectionItem, SelectionViewParams,
};
use crate::commands::CommandReply;
use crate::model_facts::entry_badges;
use crate::render::renderable::Renderable;

const MODEL: &str = "model";
const USE_MODEL: &str = "use";

pub(super) struct ModelPicker {
    provider_id: String,
    tx: AppEventSender,
    stage: Stage,
    complete: bool,
}

enum Stage {
    Listing,
    List {
        list: Box<ListSelectionView>,
        /// The ids of the models listed, in the list's order.
        models: Vec<String>,
    },
    Entry(Box<FormView>),
}

impl ModelPicker {
    pub(super) fn new(provider_id: String, tx: AppEventSender) -> Self {
        Self {
            provider_id,
            tx,
            stage: Stage::Listing,
            complete: false,
        }
    }

    fn listed(&mut self, listed: ProviderModels) {
        let mut items = Vec::new();
        let note = match &listed.result {
            Ok(models) => {
                items.extend(models.iter().map(|model| SelectionItem {
                    name: model.id.clone(),
                    description: describe(model),
                    is_current: listed.current.as_deref() == Some(model.id.as_str()),
                    search_value: Some(model.id.clone()),
                    ..Default::default()
                }));
                None
            }
            Err(error) => Some(Line::from(
                format!("The models were not listed: {error}").dim(),
            )),
        };
        let selected = items.iter().position(|item| item.is_current);
        items.push(SelectionItem {
            name: "Enter a model id…".to_string(),
            description: Some("one the Provider does not list".to_string()),
            ..Default::default()
        });
        let list = ListSelectionView::new(
            SelectionViewParams {
                title: Some("Choose the model".to_string()),
                subtitle: Some(format!(
                    "{} becomes the default; new sessions run this model.",
                    listed.name
                )),
                footer_note: note,
                footer_hint: Some(Line::from("enter choose · esc back")),
                items,
                is_searchable: true,
                search_placeholder: Some("Type to search models".to_string()),
                initial_selected_idx: selected,
                ..Default::default()
            },
            self.tx.clone(),
        );
        let models = listed
            .result
            .map(|models| models.into_iter().map(|model| model.id).collect())
            .unwrap_or_default();
        self.stage = Stage::List {
            list: Box::new(list),
            models,
        };
    }

    /// `model` on this Provider, saved as the model policy's, and the page
    /// closed.
    fn choose(&mut self, model: String) {
        let values = Map::from_iter([
            ("defaultProviderId".to_string(), json!(self.provider_id)),
            ("model".to_string(), json!(model)),
        ]);
        self.tx
            .send(AppEvent::Workspace(WorkspaceEvent::SettingsAction(
                SettingsPageAction {
                    collection: "modelPolicy".to_string(),
                    record: Some("policy".to_string()),
                    action: "save".to_string(),
                    values,
                },
            )));
        self.complete = true;
    }

    fn content(&self) -> &dyn Renderable {
        match &self.stage {
            Stage::Listing => &LISTING,
            Stage::List { list, .. } => list.as_ref(),
            Stage::Entry(form) => form.as_ref(),
        }
    }
}

/// What the page says while the models are listed.
struct Listing;

const LISTING: Listing = Listing;

impl Renderable for Listing {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(vec![
            Line::from("Choose the model".bold()),
            Line::from("Listing the models… esc closes".dim()),
        ])
        .render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        2
    }
}

impl BottomPaneView for ModelPicker {
    fn handle_key_event(&mut self, key: KeyEvent) {
        match &mut self.stage {
            Stage::Listing => self.complete = key.code == KeyCode::Esc,
            Stage::List { list, models } => {
                list.handle_key_event(key);
                match list.take_last_selected_index() {
                    Some(index) => match models.get(index).cloned() {
                        Some(model) => self.choose(model),
                        None => {
                            self.stage = Stage::Entry(Box::new(FormView::new(
                                "Enter a model id",
                                vec![
                                    FormItem::text(MODEL, "Model", "").placeholder("gpt-5.1"),
                                    FormItem::action(USE_MODEL, "Use this model"),
                                ],
                            )))
                        }
                    },
                    None => self.complete = list.is_complete(),
                }
            }
            Stage::Entry(form) => match form.handle_key(key) {
                FormEvent::Cancel => self.complete = true,
                FormEvent::Action(_) => {
                    let model = form.text(MODEL).trim().to_string();
                    if model.is_empty() {
                        form.set_error(MODEL, "Enter the model's id.");
                    } else {
                        self.choose(model);
                    }
                }
                FormEvent::None => {}
            },
        }
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        match &mut self.stage {
            Stage::Entry(form) => form.handle_paste(&pasted),
            _ => false,
        }
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match reply {
            CommandReply::ProviderModels(listed) if listed.provider_id == self.provider_id => {
                if matches!(self.stage, Stage::Listing) {
                    self.listed(listed);
                }
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

impl Renderable for ModelPicker {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.content().render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.content().desired_height(width)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        match &self.stage {
            Stage::Entry(form) => form.cursor_pos(area),
            _ => None,
        }
    }
}

/// An entry's name, if it has one, and what it reports of itself.
fn describe(model: &ModelCatalogEntry) -> Option<String> {
    let parts: Vec<String> = model
        .display_name
        .iter()
        .cloned()
        .chain(entry_badges(model))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}
