//! `/model`'s view: open before anything is heard, then the session's
//! model pinned first and a section per enabled Provider, in a fixed
//! order, each pending until its listing answers. A failed Provider is a
//! row that Enter lists again. Under the list: what is still listed, the
//! selected model's facts with their sources, and the keys.

use hanafubuki_agent::{AgentListedModel, AgentProviderModelCatalogResult, ModelMetadata};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::agent_bridge::model_list::{ModelListEvent, ModelListStart};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{
    render_menu_surface, BottomPaneView, CancellationEvent, ListSelectionView, LoadingLine,
    SelectionItem, SelectionViewParams,
};
use crate::commands::subagents::single_line;
use crate::commands::CommandReply;
use crate::model_facts::{fact_cells, fact_details, row_facts, FACTS_HEADER};
use crate::render::renderable::{ColumnRenderable, Renderable};
use crate::wrapping::word_wrap_lines;

const TITLE: &str = "Select Model";
const SUBTITLE: &str = "Switch the model of this session.";

pub(super) struct ModelListView {
    /// This view's number: answers for another are dropped.
    listing: u64,
    tx: AppEventSender,
    loading: LoadingLine,
    state: State,
    complete: bool,
}

enum State {
    /// Nothing heard yet.
    Opened,
    /// The Providers were not read: nothing is listed.
    Unread(String),
    Listing(Box<Listing>),
}

struct Listing {
    start: ModelListStart,
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
}

impl Row {
    fn provider_id(&self) -> &str {
        match self {
            Row::Model { provider_id, .. }
            | Row::Pending { provider_id, .. }
            | Row::Failed { provider_id, .. } => provider_id,
        }
    }

    fn model(&self) -> Option<&str> {
        match self {
            Row::Model { model, .. } => Some(model),
            Row::Pending { .. } | Row::Failed { .. } => None,
        }
    }
}

/// What a key did that the view acts on.
enum Chosen {
    Model { provider_id: String, model: String },
    Retry(String),
}

impl ModelListView {
    pub(super) fn new(listing: u64, tx: AppEventSender, loading: LoadingLine) -> Self {
        Self {
            listing,
            tx,
            loading,
            state: State::Opened,
            complete: false,
        }
    }

    fn heard(&mut self, event: ModelListEvent) {
        let opened = matches!(self.state, State::Opened);
        match event {
            ModelListEvent::Started(start) if opened => {
                self.state = State::Listing(Box::new(Listing::new(*start, &self.tx)));
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
            State::Opened | State::Unread(_) => &HEADING,
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
        let State::Listing(listing) = &self.state else {
            return vec![Line::from("esc close".dim())];
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
                listing.choose_hint(),
            ),
            Some(Row::Failed { name, error, .. }) => (
                Some(format!("{name}: not listed: {}", single_line(error))),
                "enter lists again · esc close".to_string(),
            ),
            Some(Row::Pending { .. }) | None => (None, listing.choose_hint()),
        };
        detail
            .map(Line::from)
            .into_iter()
            .chain([Line::from(hint.dim())])
            .collect()
    }

    /// The lines under the list at `width`, wrapped, but the loading line,
    /// which only drawing builds: it asks for the next frame.
    fn footer(&self, width: u16, draw: bool) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let mut notes = self.notes();
        match self.status() {
            Some(Status::Loading(text)) if draw => lines.push(self.loading.line(&text)),
            Some(Status::Loading(_)) => lines.push(Line::default()),
            Some(Status::Said(text)) => notes.insert(0, Line::from(text)),
            None => {}
        }
        lines.extend(wrapped(notes, width));
        lines
    }
}

/// What the line under the list is.
enum Status {
    /// Something is still listed: the moving loading line.
    Loading(String),
    /// Nothing more will be.
    Said(String),
}

impl Listing {
    fn new(start: ModelListStart, tx: &AppEventSender) -> Self {
        let sections = start
            .providers
            .iter()
            .map(|(provider_id, name)| Section {
                provider_id: provider_id.clone(),
                name: name.clone(),
                state: SectionState::Pending,
            })
            .collect();
        let header: Box<dyn Renderable> = match &start.policy_note {
            Some(note) => Box::new(ColumnRenderable::with([
                Box::new(Line::from(TITLE.bold())) as Box<dyn Renderable>,
                Box::new(
                    Paragraph::new(Line::from(format!("{SUBTITLE} {note}").dim()))
                        .wrap(Wrap { trim: false }),
                ),
            ])),
            None => Box::new(()),
        };
        let (title, subtitle) = match start.policy_note {
            Some(_) => (None, None),
            None => (Some(TITLE.to_string()), Some(SUBTITLE.to_string())),
        };
        let list = ListSelectionView::new(
            SelectionViewParams {
                title,
                subtitle,
                header,
                is_searchable: true,
                search_placeholder: Some("Type to search models".to_string()),
                facts_header: Some(FACTS_HEADER.to_string()),
                ..Default::default()
            },
            tx.clone(),
        );
        let mut listing = Self {
            start,
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
        let rows = rows(&self.start, &self.sections);
        let selected = selected
            .and_then(|(provider_id, model)| {
                rows.iter()
                    .position(|row| {
                        row.provider_id() == provider_id && row.model() == model.as_deref()
                    })
                    .or_else(|| rows.iter().position(|row| row.provider_id() == provider_id))
            })
            .or_else(|| {
                rows.iter()
                    .position(|row| !matches!(row, Row::Pending { .. }))
            });
        self.shows_unknown = rows.iter().any(|row| match row {
            Row::Model { metadata, .. } => {
                let cells = fact_cells(metadata);
                cells.window == "?" || cells.thinking == "?" || cells.vision == "?"
            }
            Row::Pending { .. } | Row::Failed { .. } => false,
        });
        self.list
            .replace_items(rows.iter().map(item).collect(), selected);
        self.rows = rows;
    }

    fn choose_hint(&self) -> String {
        let mut hint = "enter choose · esc close · ? not known".to_string();
        if self.shows_unknown && !self.start.models_dev {
            hint.push_str(" · models.dev can fill these: /config modelMetadata");
        }
        hint
    }
}

/// The rows, in their fixed order: the session's model, then each
/// Provider's section in the facade's order, its models in its listing's
/// (the session's left out), or one row saying it is listing or failed.
fn rows(start: &ModelListStart, sections: &[Section]) -> Vec<Row> {
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
                    }),
            ),
        }
    }
    rows
}

fn item(row: &Row) -> SelectionItem {
    match row {
        Row::Model {
            provider,
            model,
            display_name,
            metadata,
            current,
            ..
        } => SelectionItem {
            name: model.clone(),
            description: Some(provider.clone()),
            is_current: *current,
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
        },
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
    }
}

fn wrapped(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.saturating_sub(2).max(1));
    lines
        .into_iter()
        .flat_map(|line| word_wrap_lines([line], width))
        .collect()
}

/// The title and subtitle, before the list is known.
struct Heading;

const HEADING: Heading = Heading;

impl Renderable for Heading {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let content = render_menu_surface(area, buf);
        Paragraph::new(vec![Line::from(TITLE.bold()), Line::from(SUBTITLE.dim())])
            .render(content, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        4
    }
}

impl BottomPaneView for ModelListView {
    fn handle_key_event(&mut self, key: KeyEvent) {
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
                Row::Pending { .. } => None,
            }
        });
        self.complete = listing.list.is_complete();
        match chosen {
            Some(Chosen::Model { provider_id, model }) => {
                self.tx
                    .send(AppEvent::Op(Op::Command(AgentCommand::SetModel {
                        provider_id,
                        model,
                    })));
                self.complete = true;
            }
            Some(Chosen::Retry(provider_id)) => self.retry(provider_id),
            None => {}
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
        if area.is_empty() {
            return;
        }
        let footer = self.footer(area.width, true);
        let body = Rect {
            height: area.height.saturating_sub(footer.len() as u16),
            ..area
        };
        self.body().render(body, buf);
        for (y, line) in (body.bottom()..area.bottom()).zip(footer) {
            line.render(
                Rect::new(area.x + 2, y, area.width.saturating_sub(2), 1),
                buf,
            );
        }
    }
}
