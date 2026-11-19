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
    /// A label the caller keeps beside the item (the status line's place).
    pub(crate) note: Option<String>,
}

/// The preview of the items as they are, wrapped to the width it is given:
/// a line each, none when there is nothing to show (a caller drawing two
/// sides returns two).
pub(crate) type PreviewCallback = Box<dyn Fn(&[MultiSelectItem], u16) -> Vec<Line<'static>>>;
/// Told of every toggle and move.
pub(crate) type ChangeCallback = Box<dyn Fn(&[MultiSelectItem])>;
/// Told the items turned on, in order, on Enter; their ids are what the
/// setting keeps, and a caller's own note goes beside them.
pub(crate) type ConfirmCallback = Box<dyn Fn(&[MultiSelectItem])>;
/// Told of Esc and Ctrl-C.
pub(crate) type CancelCallback = Box<dyn Fn()>;
/// Told `key` when it is not one of the list's own, with the items as they
/// are and the highlighted one; `true` when it was taken.
pub(crate) type ExtraKeyCallback =
    Box<dyn Fn(&mut Vec<MultiSelectItem>, Option<usize>, KeyEvent) -> bool>;

pub(crate) struct MultiSelectPicker {
    buttons: super::ActionButtons,
    title: String,
    subtitle: Option<String>,
    items: Vec<MultiSelectItem>,
    state: ScrollState,
    complete: bool,
    on_preview: Option<PreviewCallback>,
    on_change: Option<ChangeCallback>,
    on_confirm: Option<ConfirmCallback>,
    on_cancel: Option<CancelCallback>,
    on_extra_key: Option<ExtraKeyCallback>,
    /// Keys of a caller's own, after the list's in the footer's hint.
    extra_keys: Option<String>,
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
            buttons: Default::default(),
            title,
            subtitle,
            items,
            state,
            complete: false,
            on_preview: None,
            on_change: None,
            on_confirm: None,
            on_cancel: None,
            on_extra_key: None,
            extra_keys: None,
        }
    }

    pub(crate) fn on_preview(mut self, callback: PreviewCallback) -> Self {
        // The width is known only when drawn (`footer_lines`).
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

    pub(crate) fn on_extra_key(mut self, callback: ExtraKeyCallback) -> Self {
        self.on_extra_key = Some(callback);
        self
    }

    /// `hint`, a caller's own keys, said after the list's in the footer.
    pub(crate) fn with_keys_hint(mut self, hint: &str) -> Self {
        self.extra_keys = Some(hint.to_string());
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
        if let Some(on_change) = &self.on_change {
            on_change(&self.items);
        }
    }

    fn confirm(&mut self) {
        let chosen: Vec<MultiSelectItem> = self
            .items
            .iter()
            .filter(|item| item.enabled)
            .cloned()
            .collect();
        if let Some(on_confirm) = &self.on_confirm {
            on_confirm(&chosen);
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
                    description: match (&item.note, &item.description) {
                        (Some(note), Some(description)) => Some(format!("{note} · {description}")),
                        (Some(note), None) => Some(note.clone()),
                        (None, description) => description.clone(),
                    },
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

    /// The preview wrapped to `width`, then a blank row for the keys.
    fn footer_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = match &self.on_preview {
            Some(preview) => preview(&self.items, width),
            None => Vec::new(),
        };
        // The keys' own row is drawn by `ActionButtons` in `render`, which
        // knows where each lands.
        lines.push(Line::default());
        lines
    }
}

impl BottomPaneView for MultiSelectPicker {
    fn pointer_key(
        &mut self,
        mouse: ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyCode> {
        self.buttons
            .key(mouse)
            .or_else(|| self.state.pointer(mouse, KeyCode::Char(' ')))
    }

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
            _ => {
                let Some(on_extra_key) = &self.on_extra_key else {
                    return;
                };
                if on_extra_key(&mut self.items, self.state.selected_idx, key) {
                    self.changed();
                }
            }
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
        2 + header
            + 1
            + self.rows_height(rows_width)
            + self.footer_lines(width.saturating_sub(2)).len() as u16
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let footer = self.footer_lines(area.width.saturating_sub(2));
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
        let buttons = Rect::new(
            footer_area.x,
            footer_area.y.saturating_add(footer.len() as u16 - 1),
            footer_area.width,
            1,
        )
        .intersection(footer_area);
        let mut footer = footer;
        footer.pop();
        Paragraph::new(footer).render(footer_area, buf);
        let actions = vec![
            ("Space toggle", KeyCode::Char(' ')),
            ("← earlier", KeyCode::Left),
            ("→ later", KeyCode::Right),
            ("Enter confirm", KeyCode::Enter),
            ("Esc cancel", KeyCode::Esc),
        ];
        self.buttons.render(buttons, buf, &actions);
        // A caller's own keys are named after these, without a button of
        // their own: the footer says what the view takes beyond its list,
        // and a click has nothing to answer.
        if let Some(extra) = &self.extra_keys {
            let rest = Rect {
                x: buttons.x,
                y: buttons.y,
                width: footer_area.width,
                height: 1,
            };
            let used: u16 = actions.iter().fold(buttons.x, |x, (label, _)| {
                let width = Line::from(label.dim()).width() as u16;
                x.saturating_add(width).saturating_add(2)
            }) - buttons.x;
            let line = Line::from(format!(" {extra}").dim());
            let width = line.width() as u16;
            line.render(
                Rect {
                    x: rest.x.saturating_add(used),
                    width: width.min(rest.width.saturating_sub(used)),
                    ..rest
                },
                buf,
            );
        }
    }
}

#[cfg(test)]
mod tests;
