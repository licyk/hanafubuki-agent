//! `/model`'s view: open before anything is heard, then the session's
//! model pinned first and a section per enabled Provider, in a fixed
//! order, each pending until its listing answers. A failed Provider is a
//! row that Enter lists again. Under the list: what is still listed, the
//! selected model's facts with their sources, and the keys.
//!
//! The same list adds a model's own settings (`Model overrides`): it then
//! leaves out nothing, marks the models that have settings already, and
//! ends with a row that types in a pair its Providers do not list.

use hanafubuki_agent::{AgentListedModel, AgentProviderModelCatalogResult, ModelMetadata};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::entry;
use super::footer::{footer, notes, render_footed, Status};
use super::purpose::Purpose;

use crate::agent_bridge::model_list::{ModelListEvent, ModelListStart};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{
    render_menu_surface, BottomPaneView, CancellationEvent, FormEvent, FormView, ListSelectionView,
    LoadingLine, SelectionItem, SelectionViewParams,
};
use crate::commands::subagents::single_line;
use crate::commands::CommandReply;
use crate::model_facts::{choose_hint, fact_cells, fact_details, row_facts, FACTS_HEADER};
use crate::render::renderable::{ColumnRenderable, Renderable};

pub(super) struct ModelListView {
    /// This view's number: answers for another are dropped.
    listing: u64,
    purpose: Purpose,
    tx: AppEventSender,
    loading: LoadingLine,
    /// The title and subtitle, before the list is known.
    heading: Heading,
    state: State,
    complete: bool,
}

enum State {
    /// Nothing heard yet.
    Opened,
    /// The Providers were not read: nothing is listed.
    Unread(String),
    Listing(Box<Listing>),
    /// A Provider and model typed in.
    Entry(Box<FormView>),
}

struct Listing {
    start: ModelListStart,
    /// Whether a Provider and model can be typed in.
    names_pair: bool,
    sections: Vec<Section>,
    list: ListSelectionView,
    /// What each of `list`'s items is, in its order.
    rows: Vec<Row>,
    /// Some row's facts say `?`.
    shows_unknown: bool,
}

/// One Provider's models.
struct Section {
    provider_id: String,
    name: String,
    state: SectionState,
}

enum SectionState {
    Pending,
    Listed(Vec<AgentListedModel>),
    Failed(String),
}

/// What a row of the list is.
#[derive(Clone)]
enum Row {
    Model {
        provider_id: String,
        /// Its Provider's name.
        provider: String,
        model: String,
        display_name: Option<String>,
        metadata: ModelMetadata,
        current: bool,
        /// The model already has settings of its own.
        has_settings: bool,
    },
    Pending {
        provider_id: String,
        name: String,
    },
    Failed {
        provider_id: String,
        name: String,
        error: String,
    },
    /// A Provider and model typed in.
    Entry,
}

impl Row {
    fn provider_id(&self) -> &str {
        match self {
            Row::Model { provider_id, .. }
            | Row::Pending { provider_id, .. }
            | Row::Failed { provider_id, .. } => provider_id,
            Row::Entry => "",
        }
    }

    fn model(&self) -> Option<&str> {
        match self {
            Row::Model { model, .. } => Some(model),
            Row::Pending { .. } | Row::Failed { .. } | Row::Entry => None,
        }
    }
}

/// What a key did that the view acts on.
enum Chosen {
    Model { provider_id: String, model: String },
    Retry(String),
    Entry,
}

impl ModelListView {
    pub(super) fn new(
        listing: u64,
        purpose: Purpose,
        tx: AppEventSender,
        loading: LoadingLine,
    ) -> Self {
        let heading = Heading {
            title: purpose.title(),
            subtitle: purpose.subtitle(),
        };
        Self {
            listing,
            purpose,
            tx,
            loading,
            heading,
            state: State::Opened,
            complete: false,
        }
    }

    fn heard(&mut self, event: ModelListEvent) {
        let opened = matches!(self.state, State::Opened);
        match event {
            ModelListEvent::Started(start) if opened => {
                self.state =
                    State::Listing(Box::new(Listing::new(*start, &self.purpose, &self.tx)));
            }
            ModelListEvent::Unread(error) if opened => self.state = State::Unread(error),
            ModelListEvent::Listed(result) => {
                if let State::Listing(listing) = &mut self.state {
                    listing.listed(result);
                }
            }
            ModelListEvent::Started(_) | ModelListEvent::Unread(_) => {}
        }
    }

    /// The failed Provider `provider_id` listed again.
    fn retry(&mut self, provider_id: String) {
        let State::Listing(listing) = &mut self.state else {
            return;
        };
        if listing.answered() == listing.sections.len() {
            self.loading.restart();
        }
        if let Some(section) = listing.section(&provider_id) {
            section.state = SectionState::Pending;
        }
        listing.refresh();
        self.tx
            .send(AppEvent::Op(Op::Command(AgentCommand::ListModels {
                listing: self.listing,
                provider_ids: Some(vec![provider_id]),
            })));
    }

    /// What is drawn over the lines under it.
    fn body(&self) -> &dyn Renderable {
        match &self.state {
            State::Listing(listing) => &listing.list,
            State::Entry(form) => form.as_ref(),
            State::Opened | State::Unread(_) => &self.heading,
        }
    }

    /// What the line under the list says: what is still listed (`None`
    /// once nothing is), or why nothing is.
    fn status(&self) -> Option<Status> {
        match &self.state {
            State::Opened => Some(Status::Loading("Listing models…".to_string())),
            State::Unread(error) => Some(Status::Said(format!(
                "The Providers were not read: {}",
                single_line(error)
            ))),
            // The form says what it waits for.
            State::Entry(_) => None,
            State::Listing(listing) if listing.sections.is_empty() => Some(Status::Said(
                "No Provider is enabled: /providers adds one.".to_string(),
            )),
            State::Listing(listing) => {
                let (answered, total) = (listing.answered(), listing.sections.len());
                let providers = if total == 1 { "Provider" } else { "Providers" };
                (answered < total).then(|| {
                    Status::Loading(format!(
                        "Listing models… {answered} of {total} {providers} answered"
                    ))
                })
            }
        }
    }

    /// The selected row's detail and the keys, before they are wrapped.
    fn notes(&self) -> Vec<Line<'static>> {
        if matches!(self.state, State::Entry(_)) {
            return Vec::new();
        }
        let State::Listing(listing) = &self.state else {
            return vec![Line::from(self.purpose.esc().dim())];
        };
        let selected = listing
            .list
            .selected_index()
            .and_then(|index| listing.rows.get(index));
        let (detail, hint) = match selected {
            Some(Row::Model {
                provider,
                model,
                metadata,
                ..
            }) => (
                Some(fact_details(model, provider, metadata)),
                listing.choose_hint(&self.purpose),
            ),
            Some(Row::Failed { name, error, .. }) => (
                Some(format!("{name}: not listed: {}", single_line(error))),
                format!("enter lists again · {}", self.purpose.esc()),
            ),
            Some(Row::Entry) => (None, format!("enter type a pair · {}", self.purpose.esc())),
            Some(Row::Pending { .. }) | None => (None, listing.choose_hint(&self.purpose)),
        };
        notes(detail, hint)
    }

    fn footer(&self, width: u16, draw: bool) -> Vec<Line<'static>> {
        footer(&self.loading, self.status(), self.notes(), width, draw)
    }
}

impl Listing {
    fn new(start: ModelListStart, purpose: &Purpose, tx: &AppEventSender) -> Self {
        let sections = start
            .providers
            .iter()
            .map(|(provider_id, name)| Section {
                provider_id: provider_id.clone(),
                name: name.clone(),
                state: SectionState::Pending,
            })
            .collect();
        let (title, subtitle) = (purpose.title(), purpose.subtitle());
        let header: Box<dyn Renderable> = match &start.policy_note {
            Some(note) => Box::new(ColumnRenderable::with([
                Box::new(Line::from(title.bold())) as Box<dyn Renderable>,
                Box::new(
                    Paragraph::new(Line::from(format!("{subtitle} {note}").dim()))
                        .wrap(Wrap { trim: false }),
                ),
            ])),
            None => Box::new(()),
        };
        let (title, subtitle) = match start.policy_note {
            Some(_) => (None, None),
            None => (Some(title.to_string()), Some(subtitle.to_string())),
        };
        let list = ListSelectionView::new(
            SelectionViewParams {
                title,
                subtitle,
                header,
                is_searchable: true,
                search_placeholder: Some("Type to search models".to_string()),
                facts_header: Some(["Model".into(), "Provider".into(), FACTS_HEADER.into()]),
                ..Default::default()
            },
            tx.clone(),
        );
        let mut listing = Self {
            start,
            names_pair: purpose.names_a_pair(),
            sections,
            list,
            rows: Vec::new(),
            shows_unknown: false,
        };
        listing.refresh();
        listing
    }

    fn section(&mut self, provider_id: &str) -> Option<&mut Section> {
        self.sections
            .iter_mut()
            .find(|section| section.provider_id == provider_id)
    }

    fn answered(&self) -> usize {
        self.sections
            .iter()
            .filter(|section| !matches!(section.state, SectionState::Pending))
            .count()
    }

    /// One Provider's listing, in its section's place.
    fn listed(&mut self, result: AgentProviderModelCatalogResult) {
        let Some(section) = self.section(&result.provider_id) else {
            return;
        };
        section.state = match result.error {
            Some(error) => SectionState::Failed(error),
            None => SectionState::Listed(result.models),
        };
        self.refresh();
    }

    /// The rows built again from what is known, the selected one kept by
    /// its Provider and model, else the first of its Provider's, else the
    /// first that can be chosen.
    fn refresh(&mut self) {
        let selected = self
            .list
            .selected_index()
            .and_then(|index| self.rows.get(index))
            .map(|row| {
                (
                    row.provider_id().to_string(),
                    row.model().map(str::to_string),
                )
            });
        let rows = rows(&self.start, &self.sections, self.names_pair);
        let selected = selected
            .and_then(|(provider_id, model)| {
                rows.iter()
                    .position(|row| {
                        row.provider_id() == provider_id && row.model() == model.as_deref()
                    })
                    .or_else(|| rows.iter().position(|row| row.provider_id() == provider_id))
            })
            .or_else(|| rows.iter().position(|row| choosable(row, self.names_pair)));
        self.shows_unknown = rows.iter().any(|row| match row {
            Row::Model { metadata, .. } => fact_cells(metadata).has_unknown(),
            _ => false,
        });
        let items = rows.iter().map(|row| item(row, self.names_pair)).collect();
        self.list.replace_items(items, selected);
        self.rows = rows;
    }

    fn choose_hint(&self, purpose: &Purpose) -> String {
        choose_hint(
            purpose.enter(),
            purpose.esc(),
            self.shows_unknown,
            self.start.models_dev,
        )
    }
}

/// Whether Enter takes `row`: a model that can be chosen, a Provider
/// listed again, or the pair typed in.
fn choosable(row: &Row, names_pair: bool) -> bool {
    match row {
        Row::Model { has_settings, .. } => !(names_pair && *has_settings),
        Row::Failed { .. } | Row::Entry => true,
        Row::Pending { .. } => false,
    }
}

/// The rows, in their fixed order: the session's model, then each
/// Provider's section in the facade's order, its models in its listing's
/// (the session's left out), or one row saying it is listing or failed;
/// the pair typed in, where it can be, last.
fn rows(start: &ModelListStart, sections: &[Section], names_pair: bool) -> Vec<Row> {
    let pinned = start.current.as_ref().map(|current| {
        let listed = sections
            .iter()
            .filter(|section| section.provider_id == current.provider_id)
            .find_map(|section| match &section.state {
                SectionState::Listed(models) => models
                    .iter()
                    .find(|listed| listed.entry.id == current.model),
                SectionState::Pending | SectionState::Failed(_) => None,
            });
        Row::Model {
            provider_id: current.provider_id.clone(),
            provider: current.provider.clone(),
            model: current.model.clone(),
            display_name: listed.and_then(|listed| listed.entry.display_name.clone()),
            metadata: listed.map_or_else(
                || current.metadata.clone(),
                |listed| listed.metadata.clone(),
            ),
            current: true,
            has_settings: start.has_settings(&current.provider_id, &current.model),
        }
    });
    let is_pinned = |provider_id: &str, model: &str| {
        start
            .current
            .as_ref()
            .is_some_and(|current| current.provider_id == provider_id && current.model == model)
    };
    let mut rows: Vec<Row> = pinned.into_iter().collect();
    for section in sections {
        let provider_id = section.provider_id.clone();
        let name = section.name.clone();
        match &section.state {
            SectionState::Pending => rows.push(Row::Pending { provider_id, name }),
            SectionState::Failed(error) => rows.push(Row::Failed {
                provider_id,
                name,
                error: error.clone(),
            }),
            SectionState::Listed(models) => rows.extend(
                models
                    .iter()
                    .filter(|listed| !is_pinned(&provider_id, &listed.entry.id))
                    .map(|listed| Row::Model {
                        provider_id: provider_id.clone(),
                        provider: name.clone(),
                        model: listed.entry.id.clone(),
                        display_name: listed.entry.display_name.clone(),
                        metadata: listed.metadata.clone(),
                        current: false,
                        has_settings: start.has_settings(&provider_id, &listed.entry.id),
                    }),
            ),
        }
    }
    if names_pair {
        rows.push(Row::Entry);
    }
    rows
}

fn item(row: &Row, names_pair: bool) -> SelectionItem {
    match row {
        Row::Model {
            provider,
            model,
            display_name,
            metadata,
            current,
            has_settings,
            ..
        } => {
            let taken = names_pair && *has_settings;
            SelectionItem {
                // A model with settings is named as such rather than
                // described: a description gives way to the fact columns.
                name: match taken {
                    true => format!("{model} (has settings)"),
                    false => model.clone(),
                },
                description: Some(provider.clone()),
                is_current: *current,
                is_disabled: taken,
                search_value: Some(
                    [
                        Some(model.as_str()),
                        display_name.as_deref(),
                        Some(provider),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" "),
                ),
                facts: Some(row_facts(metadata)),
                ..Default::default()
            }
        }
        Row::Pending { name, .. } => SelectionItem {
            name: name.clone(),
            description: Some("listing…".to_string()),
            is_disabled: true,
            search_value: Some(name.clone()),
            ..Default::default()
        },
        Row::Failed { name, error, .. } => SelectionItem {
            name: name.clone(),
            description: Some(format!("not listed: {}", single_line(error))),
            search_value: Some(name.clone()),
            ..Default::default()
        },
        Row::Entry => SelectionItem {
            name: entry::LABEL.to_string(),
            search_value: Some(entry::LABEL.to_string()),
            ..Default::default()
        },
    }
}

/// The title and subtitle, before the list is known.
struct Heading {
    title: &'static str,
    subtitle: &'static str,
}

impl Renderable for Heading {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let content = render_menu_surface(area, buf);
        Paragraph::new(vec![
            Line::from(self.title.bold()),
            Line::from(self.subtitle.dim()),
        ])
        .render(content, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        4
    }
}

impl BottomPaneView for ModelListView {
    fn pointer_key(
        &mut self,
        mouse: ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyCode> {
        match &mut self.state {
            State::Listing(listing) => listing.list.pointer_key(mouse),
            State::Entry(form) => form.pointer_key(mouse),
            _ => None,
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        if let State::Entry(form) = &mut self.state {
            match form.handle_key(key) {
                FormEvent::Cancel => self.complete = true,
                FormEvent::Action(entry::TAKE) => {
                    if let Some(pair) = entry::pair(form) {
                        let (provider_id, model) = entry::split(&pair);
                        self.purpose.choose(&self.tx, provider_id, model);
                        self.complete = true;
                    }
                }
                FormEvent::Action(_) | FormEvent::None => {}
            }
            return;
        }
        let State::Listing(listing) = &mut self.state else {
            self.complete = key.code == KeyCode::Esc;
            return;
        };
        listing.list.handle_key_event(key);
        let chosen = listing.list.take_last_selected_index().and_then(|index| {
            match listing.rows.get(index)? {
                Row::Model {
                    provider_id, model, ..
                } => Some(Chosen::Model {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                }),
                Row::Failed { provider_id, .. } => Some(Chosen::Retry(provider_id.clone())),
                Row::Entry => Some(Chosen::Entry),
                Row::Pending { .. } => None,
            }
        });
        self.complete = listing.list.is_complete();
        match chosen {
            Some(Chosen::Model { provider_id, model }) => {
                self.purpose.choose(&self.tx, &provider_id, &model);
                self.complete = true;
            }
            Some(Chosen::Retry(provider_id)) => self.retry(provider_id),
            Some(Chosen::Entry) => self.state = State::Entry(Box::new(entry::form())),
            None => {}
        }
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        match &mut self.state {
            State::Entry(form) => form.handle_paste(&pasted),
            _ => false,
        }
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        let CommandReply::ModelList(update) = reply else {
            return Some(reply);
        };
        if update.listing == self.listing {
            self.heard(update.event);
        }
        None
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for ModelListView {
    fn desired_height(&self, width: u16) -> u16 {
        let footer = self.footer(width, false).len() as u16;
        self.body().desired_height(width).saturating_add(footer)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        render_footed(self.body(), self.footer(area.width, true), area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        match &self.state {
            State::Entry(form) => form.cursor_pos(area),
            _ => None,
        }
    }
}
