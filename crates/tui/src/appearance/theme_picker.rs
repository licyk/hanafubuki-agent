// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/theme`'s picker: the bundled themes and the custom `.tmTheme` files
//! of the themes directory. Moving through the list makes the highlighted
//! theme the one in use, shown on a sample diff beside the list (under
//! it in a narrow terminal); Enter keeps it and saves `tui.theme`, Esc and
//! Ctrl-C put back the theme the picker opened with.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use syntect::highlighting::Theme;

use super::AppearanceEvent;
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, ListSelectionView, SelectionItem, SelectionViewParams,
};
use crate::diff_render::{highlight_lines, push_wrapped_diff_line, DiffLineKind};
use crate::render::highlight::{
    current_syntax_theme, list_available_themes, resolve_theme_by_name, set_syntax_theme,
    themes_dir, DEFAULT_THEME,
};
use crate::render::renderable::Renderable;

/// The sample: one line removed and one added between two kept.
const PREVIEW_ROWS: [(DiffLineKind, &str); 4] = [
    (DiffLineKind::Context, "fn greet(name: &str) -> String {"),
    (DiffLineKind::Removed, "    format!(\"Hello, {}!\", name)"),
    (DiffLineKind::Added, "    format!(\"Hello, {name}!\") // 42"),
    (DiffLineKind::Context, "}"),
];
/// The sample beside the list from this width on.
const SIDE_BY_SIDE_MIN_WIDTH: u16 = 96;

pub(crate) struct ThemePicker {
    list: ListSelectionView,
    /// The theme of each item.
    names: Vec<String>,
    /// The theme in use when the picker opened.
    original: Theme,
    previewed: Option<usize>,
}

impl ThemePicker {
    /// The picker, the saved `tui.theme` (`current`) highlighted.
    pub(crate) fn new(current: Option<&str>, tx: AppEventSender) -> Self {
        let dir = themes_dir();
        let entries = list_available_themes(dir);
        let current = current
            .filter(|name| entries.iter().any(|entry| entry.name == *name))
            .unwrap_or(DEFAULT_THEME);
        let initial = entries.iter().position(|entry| entry.name == current);
        let items = entries
            .iter()
            .map(|entry| {
                let name = entry.name.clone();
                SelectionItem {
                    name: if entry.is_custom {
                        format!("{} (custom)", entry.name)
                    } else {
                        entry.name.clone()
                    },
                    is_current: entry.name == current,
                    search_value: Some(entry.name.clone()),
                    dismiss_on_select: true,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::Appearance(AppearanceEvent::ThemeSaved(
                            name.clone(),
                        )))
                    })],
                    ..Default::default()
                }
            })
            .collect();
        let subtitle = match dir {
            Some(dir) => format!("Custom .tmTheme files go in {}.", dir.display()),
            None => "Move up and down to preview a theme.".to_string(),
        };
        let list = ListSelectionView::new(
            SelectionViewParams {
                title: Some("Select Syntax Theme".to_string()),
                subtitle: Some(subtitle),
                items,
                is_searchable: true,
                search_placeholder: Some("Type to filter themes...".to_string()),
                initial_selected_idx: initial,
                ..Default::default()
            },
            tx,
        );
        let previewed = list.selected_index();
        Self {
            list,
            names: entries.into_iter().map(|entry| entry.name).collect(),
            original: current_syntax_theme(),
            previewed,
        }
    }

    /// After a key: the highlighted theme is the one in use; a picker
    /// closed without a choice puts the first one back.
    fn follow_selection(&mut self) {
        if self.list.is_complete() {
            match self.list.take_last_selected_index() {
                Some(index) => self.preview(Some(index)),
                None => set_syntax_theme(self.original.clone()),
            }
            return;
        }
        let selected = self.list.selected_index();
        if selected != self.previewed {
            self.preview(selected);
        }
    }

    fn preview(&mut self, index: Option<usize>) {
        self.previewed = index;
        let theme = index
            .and_then(|index| self.names.get(index))
            .and_then(|name| resolve_theme_by_name(name, themes_dir()));
        if let Some(theme) = theme {
            set_syntax_theme(theme);
        }
    }
}

impl BottomPaneView for ThemePicker {
    fn handle_key_event(&mut self, key: KeyEvent) {
        self.list.handle_key_event(key);
        self.follow_selection();
    }

    fn is_complete(&self) -> bool {
        self.list.is_complete()
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        let event = self.list.on_ctrl_c();
        self.follow_selection();
        event
    }
}

impl Renderable for ThemePicker {
    fn desired_height(&self, width: u16) -> u16 {
        if width >= SIDE_BY_SIDE_MIN_WIDTH {
            let half = width / 2;
            self.list
                .desired_height(half)
                .max(PREVIEW_ROWS.len() as u16 + 2)
        } else {
            self.list.desired_height(width) + PREVIEW_ROWS.len() as u16 + 1
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if width_allows_side_by_side(area.width) {
            let list_width = area.width / 2;
            self.list.render(
                Rect {
                    width: list_width,
                    ..area
                },
                buf,
            );
            let side = Rect {
                x: area.x + list_width + 2,
                y: area.y + 1,
                width: area.width.saturating_sub(list_width + 2),
                height: area.height.saturating_sub(1),
            };
            Paragraph::new(preview_lines(side.width)).render(side, buf);
        } else {
            let list_height = self.list.desired_height(area.width).min(area.height);
            self.list.render(
                Rect {
                    height: list_height,
                    ..area
                },
                buf,
            );
            let below = Rect {
                x: area.x + 2,
                y: area.y + list_height + 1,
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(list_height + 1),
            };
            Paragraph::new(preview_lines(below.width)).render(below, buf);
        }
    }
}

fn width_allows_side_by_side(width: u16) -> bool {
    width >= SIDE_BY_SIDE_MIN_WIDTH
}

/// The sample diff in the theme in use, a row each.
pub(crate) fn preview_lines(width: u16) -> Vec<Line<'static>> {
    let texts: Vec<String> = PREVIEW_ROWS
        .iter()
        .map(|(_, text)| text.to_string())
        .collect();
    let highlighted = highlight_lines("preview.rs", &texts).unwrap_or_default();
    let mut highlighted = highlighted.into_iter();
    PREVIEW_ROWS
        .iter()
        .filter_map(|(kind, text)| {
            push_wrapped_diff_line(*kind, text, highlighted.next(), None, usize::from(width))
                .into_iter()
                .next()
        })
        .collect()
}

#[cfg(test)]
mod tests;
