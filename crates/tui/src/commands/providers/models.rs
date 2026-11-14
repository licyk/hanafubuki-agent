// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A Provider's model page: the models it lists, one of which, or an id
//! typed in, becomes the model new sessions run, with the Provider made
//! the default. The choice is a save of the model policy, which the
//! settings center runs as its own. While the models are listed it shows
//! the loading line; then each model with its fact columns, as `/model`
//! does, and under the list the selected model's facts with their sources.

use hanafubuki_agent::AgentListedModel;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};
use serde_json::{json, Map};

use crate::agent_bridge::ProviderModels;
use crate::app::{SettingsPageAction, WorkspaceEvent};
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::{
    render_menu_surface, BottomPaneView, CancellationEvent, FormEvent, FormItem, FormView,
    ListSelectionView, LoadingLine, SelectionItem, SelectionViewParams,
};
use crate::commands::model::footer::{footer, notes, render_footed, Status};
use crate::commands::subagents::single_line;
use crate::commands::CommandReply;
use crate::model_facts::{choose_hint, fact_cells, fact_details, row_facts, FACTS_HEADER};
use crate::render::renderable::{ColumnRenderable, Renderable};

const TITLE: &str = "Choose the model";
const MODEL: &str = "model";
const USE_MODEL: &str = "use";

pub(super) struct ModelPicker {
    provider_id: String,
    tx: AppEventSender,
    loading: LoadingLine,
    stage: Stage,
    complete: bool,
}

enum Stage {
    Listing,
    List(Box<Listed>),
    Entry(Box<FormView>),
}

/// The list of what the Provider listed.
struct Listed {
    list: ListSelectionView,
    /// The Provider's name.
    name: String,
    /// The models, in the list's order; the entry row follows them.
    models: Vec<AgentListedModel>,
    /// Why none were listed.
    error: Option<String>,
    /// Some model's facts say `?`.
    shows_unknown: bool,
    models_dev: bool,
}

impl ModelPicker {
    pub(super) fn new(provider_id: String, tx: AppEventSender, loading: LoadingLine) -> Self {
        Self {
            provider_id,
            tx,
            loading,
            stage: Stage::Listing,
            complete: false,
        }
    }

    fn listed(&mut self, listed: ProviderModels) {
        let (models, error) = match listed.result {
            Ok(models) => (models, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let mut items: Vec<SelectionItem> = models
            .iter()
            .map(|model| SelectionItem {
                name: model.entry.id.clone(),
                description: model.entry.display_name.clone(),
                is_current: listed.current.as_deref() == Some(model.entry.id.as_str()),
                search_value: Some(
                    [Some(&model.entry.id), model.entry.display_name.as_ref()]
                        .into_iter()
                        .flatten()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                facts: Some(row_facts(&model.metadata)),
                ..Default::default()
            })
            .collect();
        let selected = items.iter().position(|item| item.is_current);
        items.push(SelectionItem {
            name: "Enter a model id…".to_string(),
            description: Some("one the Provider does not list".to_string()),
            ..Default::default()
        });
        let list = ListSelectionView::new(
            SelectionViewParams {
                // The subtitle wraps at narrow widths, as `/model`'s does.
                header: Box::new(ColumnRenderable::with([
                    Box::new(Line::from(TITLE.bold())) as Box<dyn Renderable>,
                    Box::new(
                        Paragraph::new(Line::from(
                            format!(
                                "{} becomes the default; new sessions run this model.",
                                listed.name
                            )
                            .dim(),
                        ))
                        .wrap(Wrap { trim: false }),
                    ),
                ])),
                items,
                is_searchable: true,
                search_placeholder: Some("Type to search models".to_string()),
                initial_selected_idx: selected,
                facts_header: Some(FACTS_HEADER.to_string()),
                ..Default::default()
            },
            self.tx.clone(),
        );
        self.stage = Stage::List(Box::new(Listed {
            list,
            name: listed.name,
            shows_unknown: models
                .iter()
                .any(|model| fact_cells(&model.metadata).has_unknown()),
            models,
            error,
            models_dev: listed.models_dev,
        }));
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
            Stage::Listing => &HEADING,
            Stage::List(listed) => &listed.list,
            Stage::Entry(form) => form.as_ref(),
        }
    }

    /// The lines under the list at `width` (`footer`'s `draw`).
    fn footer(&self, width: u16, draw: bool) -> Vec<Line<'static>> {
        let (status, notes) = match &self.stage {
            // The center's own line says what Esc does.
            Stage::Listing => (
                Some(Status::Loading("Listing the models…".to_string())),
                Vec::new(),
            ),
            Stage::List(listed) => (
                listed.error.as_deref().map(|error| {
                    Status::Said(format!(
                        "The models were not listed: {}",
                        single_line(error)
                    ))
                }),
                listed.notes(),
            ),
            Stage::Entry(_) => return Vec::new(),
        };
        footer(&self.loading, status, notes, width, draw)
    }
}

impl Listed {
    /// The selected model's facts, none on the entry row, and the keys.
    fn notes(&self) -> Vec<Line<'static>> {
        let detail = self
            .list
            .selected_index()
            .and_then(|index| self.models.get(index))
            .map(|model| fact_details(&model.entry.id, &self.name, &model.metadata));
        let hint = if self.models.is_empty() {
            "enter choose · esc back".to_string()
        } else {
            choose_hint("esc back", self.shows_unknown, self.models_dev)
        };
        notes(detail, hint)
    }
}

/// The title, before the models are known.
struct Heading;

const HEADING: Heading = Heading;

impl Renderable for Heading {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let content = render_menu_surface(area, buf);
        Paragraph::new(Line::from(TITLE.bold())).render(content, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        3
    }
}

impl BottomPaneView for ModelPicker {
    fn handle_key_event(&mut self, key: KeyEvent) {
        match &mut self.stage {
            Stage::Listing => self.complete = key.code == KeyCode::Esc,
            Stage::List(listed) => {
                let list = &mut listed.list;
                list.handle_key_event(key);
                match list.take_last_selected_index() {
                    Some(index) => {
                        match listed.models.get(index).map(|model| model.entry.id.clone()) {
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
                        }
                    }
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
        render_footed(self.content(), self.footer(area.width, true), area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let footer = self.footer(width, false).len() as u16;
        self.content().desired_height(width).saturating_add(footer)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        match &self.stage {
            Stage::Entry(form) => form.cursor_pos(area),
            _ => None,
        }
    }
}
