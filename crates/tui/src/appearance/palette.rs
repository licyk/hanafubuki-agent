//! `/statusline palette`: one item's colour. The item is picked from the
//! status line's own ids or typed in, then the colour from the sixteen ANSI
//! names or typed as `#rrggbb`. The choice is saved as one
//! `tui.statusLinePalette` entry, `<item>:<color>`.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;

use super::items::{StatusLineItem, SurfaceItem};
use super::AppearanceEvent;
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::{
    render_menu_surface, BottomPaneView, CancellationEvent, FormEvent, FormItem, FormView,
    ListSelectionView, SelectionItem, SelectionViewParams,
};
use crate::render::renderable::Renderable;

const TITLE: &str = "Status line palette";
const ITEM: &str = "item";
const COLOR: &str = "color";
const TYPE_ITEM: &str = "type-item";
const TYPE_COLOR: &str = "type-color";

/// The colours a palette entry may name, as `/statusline palette` offers
/// them: the sixteen ANSI names. `#rrggbb` is typed in.
pub(crate) const COLOR_NAMES: &[&str] = &[
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "gray",
    "darkgray",
    "lightred",
    "lightgreen",
    "lightyellow",
    "lightblue",
    "lightmagenta",
    "lightcyan",
    "white",
];

/// `tui.statusLinePalette`'s entries as a colour per item id, with the
/// entries this interface cannot read: an item it does not know, or a
/// colour that is neither a name nor `#rrggbb`.
pub(crate) fn parse(
    entries: &[String],
) -> (
    std::collections::BTreeMap<String, ratatui::style::Color>,
    Vec<String>,
) {
    let mut palette = std::collections::BTreeMap::new();
    let mut bad = Vec::new();
    for entry in entries {
        let parsed = entry.split_once(':').and_then(|(id, color)| {
            let item = StatusLineItem::parse(id)?;
            let color = super::parse_color(color)?;
            Some((item, color))
        });
        match parsed {
            Some((item, color)) => {
                palette.insert(item.id().to_string(), color);
            }
            None => bad.push(entry.clone()),
        }
    }
    (palette, bad)
}

pub(crate) struct PaletteView {
    tx: AppEventSender,
    stage: Stage,
    complete: bool,
}

enum Stage {
    Item(ListSelectionView),
    ItemText(Box<FormView>),
    Color {
        item: String,
        list: ListSelectionView,
    },
    ColorText {
        item: String,
        form: Box<FormView>,
    },
}

impl PaletteView {
    pub(crate) fn new(tx: AppEventSender) -> Self {
        Self {
            stage: Stage::Item(item_list(tx.clone())),
            tx,
            complete: false,
        }
    }

    fn choose_item(&mut self, item: String) {
        self.stage = Stage::Color {
            list: color_list(&self.tx, &item),
            item,
        };
    }

    fn save(&mut self, item: String, color: String) {
        if super::parse_color(&color).is_none() {
            if let Stage::ColorText { form, .. } = &mut self.stage {
                form.set_error(COLOR, "A colour is one of the listed names, or `#rrggbb`.");
            }
            return;
        }
        self.tx
            .send(AppEvent::Appearance(AppearanceEvent::StatusLine {
                items: None,
                colors: None,
                positions: None,
                overflow: None,
                palette: Some(vec![format!("{item}:{color}")]),
                save: true,
            }));
        self.complete = true;
    }
}

fn item_row(item: &StatusLineItem) -> SelectionItem {
    SelectionItem {
        name: item.id().to_string(),
        description: Some(item.description().to_string()),
        ..Default::default()
    }
}

fn typed_item_row() -> SelectionItem {
    SelectionItem {
        name: "Type an id…".to_string(),
        description: Some("An item id this list does not name".to_string()),
        ..Default::default()
    }
}

fn item_list(tx: AppEventSender) -> ListSelectionView {
    let items = StatusLineItem::ALL
        .iter()
        .map(item_row)
        .chain(std::iter::once(typed_item_row()))
        .collect();
    ListSelectionView::new(
        SelectionViewParams {
            title: Some(TITLE.to_string()),
            subtitle: Some("Which item takes a colour?".to_string()),
            items,
            is_searchable: true,
            ..Default::default()
        },
        tx,
    )
}

fn color_row(name: &str) -> SelectionItem {
    SelectionItem {
        name: (*name).to_string(),
        description: None,
        ..Default::default()
    }
}

fn typed_color_row() -> SelectionItem {
    SelectionItem {
        name: "Type a colour…".to_string(),
        description: Some("`#rrggbb`, or a name this list does not have".to_string()),
        ..Default::default()
    }
}

fn color_list(tx: &AppEventSender, item: &str) -> ListSelectionView {
    let items = COLOR_NAMES
        .iter()
        .map(|name| color_row(name))
        .chain(std::iter::once(typed_color_row()))
        .collect();
    ListSelectionView::new(
        SelectionViewParams {
            title: Some(TITLE.to_string()),
            subtitle: Some(format!("Which colour for {item}?")),
            items,
            is_searchable: true,
            ..Default::default()
        },
        tx.clone(),
    )
}

fn item_form(value: &str) -> FormView {
    FormView::new(
        TITLE,
        vec![
            FormItem::text(ITEM, "Item", value).placeholder("model"),
            FormItem::action(TYPE_ITEM, "Choose the colour"),
        ],
    )
    .with_subtitle("An item id: `model`, `context-window` and the like.".to_string())
}

fn color_form(item: &str, value: &str) -> FormView {
    FormView::new(
        TITLE,
        vec![
            FormItem::text(COLOR, "Colour", value).placeholder("#ff8800"),
            FormItem::action(TYPE_COLOR, "Colour this item"),
        ],
    )
    .with_subtitle(format!(
        "The colour for {item}: one of the ANSI names, or `#rrggbb`."
    ))
}

impl BottomPaneView for PaletteView {
    fn handle_key_event(&mut self, key: KeyEvent) {
        match &mut self.stage {
            Stage::Item(list) => {
                list.handle_key_event(key);
                let selected = list.take_last_selected_index();
                match selected {
                    Some(index) => match StatusLineItem::ALL.get(index) {
                        Some(item) => {
                            let id = item.id().to_string();
                            self.choose_item(id);
                        }
                        None if index == StatusLineItem::ALL.len() => {
                            self.stage = Stage::ItemText(Box::new(item_form("")));
                        }
                        None => {}
                    },
                    None => self.complete = list.is_complete(),
                }
            }
            Stage::ItemText(form) => match form.handle_key(key) {
                FormEvent::Cancel => self.complete = true,
                FormEvent::Action(_) => {
                    let item = form.text(ITEM).trim().to_ascii_lowercase();
                    if item.is_empty() {
                        form.set_error(ITEM, "Enter the item's id.");
                    } else {
                        self.choose_item(item);
                    }
                }
                FormEvent::None => {}
            },
            Stage::Color { list, item } => {
                list.handle_key_event(key);
                let selected = list.take_last_selected_index();
                let item = item.clone();
                match selected {
                    Some(index) => match COLOR_NAMES.get(index) {
                        Some(name) => {
                            let color = (*name).to_string();
                            self.save(item, color);
                        }
                        None if index == COLOR_NAMES.len() => {
                            self.stage = Stage::ColorText {
                                form: Box::new(color_form(&item, "")),
                                item,
                            };
                        }
                        None => {}
                    },
                    None => self.complete = list.is_complete(),
                }
            }
            Stage::ColorText { form, item } => {
                let item = item.clone();
                match form.handle_key(key) {
                    FormEvent::Cancel => self.complete = true,
                    FormEvent::Action(_) => {
                        let color = form.text(COLOR).trim().to_ascii_lowercase();
                        if color.is_empty() {
                            form.set_error(COLOR, "Enter the colour.");
                        } else {
                            self.save(item, color);
                        }
                    }
                    FormEvent::None => {}
                }
            }
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

impl Renderable for PaletteView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let content = render_menu_surface(area, buf);
        match &self.stage {
            Stage::Item(list) | Stage::Color { list, .. } => list.render(content, buf),
            Stage::ItemText(form) | Stage::ColorText { form, .. } => form.render(content, buf),
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        match &self.stage {
            Stage::Item(list) | Stage::Color { list, .. } => list.desired_height(width),
            Stage::ItemText(form) | Stage::ColorText { form, .. } => form.desired_height(width),
        }
    }
}
