// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A list of items to turn on and off and put in order, with a line that
//! previews the result (`/statusline` and `/title`'s setup). Space toggles
//! the highlighted item, Left and Right move it, Enter confirms the items
//! turned on in their order, Esc cancels. Codex's search field and
//! non-orderable sections are left out: the lists are short.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::bottom_pane_view::BottomPaneView;
use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::{
    measure_rows_height, render_menu_surface, render_rows, GenericDisplayRow,
};
use super::CancellationEvent;
use crate::render::renderable::Renderable;

/// One item of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MultiSelectItem {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) enabled: bool,
}

/// The preview line of the items as they are.
pub(crate) type PreviewCallback = Box<dyn Fn(&[MultiSelectItem]) -> Option<Line<'static>>>;
/// Told of every toggle and move.
pub(crate) type ChangeCallback = Box<dyn Fn(&[MultiSelectItem])>;
/// Told the ids turned on, in order, on Enter.
pub(crate) type ConfirmCallback = Box<dyn Fn(&[String])>;
/// Told of Esc and Ctrl-C.
pub(crate) type CancelCallback = Box<dyn Fn()>;

pub(crate) struct MultiSelectPicker {
    title: String,
    subtitle: Option<String>,
    items: Vec<MultiSelectItem>,
    state: ScrollState,
    complete: bool,
    preview_line: Option<Line<'static>>,
    on_preview: Option<PreviewCallback>,
    on_change: Option<ChangeCallback>,
    on_confirm: Option<ConfirmCallback>,
    on_cancel: Option<CancelCallback>,
}

impl MultiSelectPicker {
    pub(crate) fn new(
        title: String,
        subtitle: Option<String>,
        items: Vec<MultiSelectItem>,
    ) -> Self {
        let mut state = ScrollState::new();
        state.clamp_selection(items.len());
        Self {
            title,
            subtitle,
            items,
            state,
            complete: false,
            preview_line: None,
            on_preview: None,
            on_change: None,
            on_confirm: None,
            on_cancel: None,
        }
    }

    pub(crate) fn on_preview(mut self, callback: PreviewCallback) -> Self {
        self.preview_line = callback(&self.items);
        self.on_preview = Some(callback);
        self
    }

    pub(crate) fn on_change(mut self, callback: ChangeCallback) -> Self {
        self.on_change = Some(callback);
        self
    }

    pub(crate) fn on_confirm(mut self, callback: ConfirmCallback) -> Self {
        self.on_confirm = Some(callback);
        self
    }

    pub(crate) fn on_cancel(mut self, callback: CancelCallback) -> Self {
        self.on_cancel = Some(callback);
        self
    }

    fn move_cursor(&mut self, down: bool) {
        let len = self.items.len();
        if down {
            self.state.move_down_wrap(len);
        } else {
            self.state.move_up_wrap(len);
        }
        self.state
            .ensure_visible(len, MAX_POPUP_ROWS.min(len.max(1)));
    }

    fn toggle_selected(&mut self) {
        let Some(item) = self
            .state
            .selected_idx
            .and_then(|index| self.items.get_mut(index))
        else {
            return;
        };
        item.enabled = !item.enabled;
        self.changed();
    }

    /// Moves the highlighted item one place, the cursor with it.
    fn move_selected_item(&mut self, down: bool) {
        let Some(index) = self.state.selected_idx else {
            return;
        };
        let target = if down {
            index + 1
        } else {
            match index.checked_sub(1) {
                Some(target) => target,
                None => return,
            }
        };
        if target >= self.items.len() {
            return;
        }
        self.items.swap(index, target);
        self.state.selected_idx = Some(target);
        self.state
            .ensure_visible(self.items.len(), MAX_POPUP_ROWS.min(self.items.len()));
        self.changed();
    }

    fn changed(&mut self) {
        self.preview_line = self
            .on_preview
            .as_ref()
            .and_then(|preview| preview(&self.items));
        if let Some(on_change) = &self.on_change {
            on_change(&self.items);
        }
    }

    fn confirm(&mut self) {
        let ids: Vec<String> = self
            .items
            .iter()
            .filter(|item| item.enabled)
            .map(|item| item.id.clone())
            .collect();
        if let Some(on_confirm) = &self.on_confirm {
            on_confirm(&ids);
        }
        self.complete = true;
    }

    fn cancel(&mut self) {
        if let Some(on_cancel) = &self.on_cancel {
            on_cancel();
        }
        self.complete = true;
    }

    fn build_rows(&self) -> Vec<GenericDisplayRow> {
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let cursor = if self.state.selected_idx == Some(index) {
                    '›'
                } else {
                    ' '
                };
                let marker = if item.enabled { 'x' } else { ' ' };
                GenericDisplayRow {
                    name: format!("{cursor} [{marker}] {}", item.name),
                    description: item.description.clone(),
                    ..Default::default()
                }
            })
            .collect()
    }

    fn header_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(self.title.clone().bold())];
        lines.extend(
            self.subtitle
                .as_ref()
                .map(|subtitle| Line::from(subtitle.clone().dim())),
        );
        lines
    }

    fn rows_height(&self, width: u16) -> u16 {
        measure_rows_height(
            &self.build_rows(),
            &self.state,
            MAX_POPUP_ROWS,
            width.saturating_sub(1),
        )
    }

    fn footer_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(preview) = &self.preview_line {
            lines.push(preview.clone());
        }
        lines.push(Line::from("space toggle · ←/→ move · enter confirm · esc cancel").dim());
        lines
    }
}

impl BottomPaneView for MultiSelectPicker {
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if !control => self.move_cursor(false),
            KeyCode::Char('p') if control => self.move_cursor(false),
            KeyCode::Down | KeyCode::Char('j') if !control => self.move_cursor(true),
            KeyCode::Char('n') if control => self.move_cursor(true),
            KeyCode::Left => self.move_selected_item(false),
            KeyCode::Right => self.move_selected_item(true),
            KeyCode::Char(' ') => self.toggle_selected(),
            KeyCode::Enter => self.confirm(),
            KeyCode::Esc => self.cancel(),
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.cancel();
        CancellationEvent::Handled
    }
}

impl Renderable for MultiSelectPicker {
    fn desired_height(&self, width: u16) -> u16 {
        let rows_width = width.saturating_sub(4);
        let header = self.header_lines().len() as u16;
        // The surface's padding, the header, a gap, the rows; then the
        // preview and the key hints.
        2 + header + 1 + self.rows_height(rows_width) + self.footer_lines().len() as u16
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let footer = self.footer_lines();
        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(footer.len() as u16)])
                .areas(area);
        let content_area = render_menu_surface(content_area, buf);
        let header = self.header_lines();
        let [header_area, _, list_area] = Layout::vertical([
            Constraint::Length(header.len() as u16),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(content_area);
        Paragraph::new(header).render(header_area, buf);
        if list_area.height > 0 {
            let rows_area = Rect {
                x: list_area.x.saturating_sub(2),
                width: list_area.width.saturating_add(2),
                ..list_area
            };
            render_rows(
                rows_area,
                buf,
                &self.build_rows(),
                &self.state,
                usize::from(rows_area.height),
                "no items",
            );
        }
        let footer_area = Rect {
            x: footer_area.x + 2,
            width: footer_area.width.saturating_sub(2),
            ..footer_area
        };
        Paragraph::new(footer).render(footer_area, buf);
    }
}

#[cfg(test)]
mod tests;
