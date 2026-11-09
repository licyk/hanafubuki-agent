// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A list of rows that a command fills and gives its own keys (Codex's
//! `config_panel/`, `memories_settings_view.rs` and `warnings_view.rs`
//! share this shape): a title and header lines, the rows, a line of key
//! hints. Up and Down (k, j, Ctrl-P, Ctrl-N) move, Esc and Ctrl-C close;
//! every other key goes to the command's [`MenuModel`] with the row it is
//! on.

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
use crate::app_event::AppEventSender;
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

/// What a key did in a [`MenuModel`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MenuKey {
    Handled,
    /// Handled, and the menu closes.
    Close,
    Ignored,
}

/// A command's menu: what it shows and what its keys do.
pub(crate) trait MenuModel {
    fn title(&self) -> String;

    /// Lines under the title.
    fn header(&self) -> Vec<Line<'static>> {
        Vec::new()
    }

    fn rows(&self) -> Vec<GenericDisplayRow>;

    /// The key hints under the rows.
    fn hints(&self) -> String;

    /// A key on the row at `selected` (`None` with no rows).
    fn on_key(&mut self, key: KeyEvent, selected: Option<usize>, tx: &AppEventSender) -> MenuKey;

    /// A reply of the command's call, or the reply handed back.
    fn on_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        Some(reply)
    }

    fn empty_message(&self) -> &'static str {
        "nothing"
    }
}

pub(crate) struct MenuView<M> {
    model: M,
    state: ScrollState,
    complete: bool,
    app_event_tx: AppEventSender,
}

impl<M: MenuModel> MenuView<M> {
    pub(crate) fn new(model: M, app_event_tx: AppEventSender) -> Self {
        let mut state = ScrollState::new();
        state.clamp_selection(model.rows().len());
        Self {
            model,
            state,
            complete: false,
            app_event_tx,
        }
    }

    fn move_cursor(&mut self, down: bool) {
        let len = self.model.rows().len();
        if down {
            self.state.move_down_wrap(len);
        } else {
            self.state.move_up_wrap(len);
        }
        self.state
            .ensure_visible(len, MAX_POPUP_ROWS.min(len.max(1)));
    }

    fn header_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(self.model.title().bold())];
        lines.extend(self.model.header());
        lines
    }

    fn rows(&self) -> Vec<GenericDisplayRow> {
        self.model
            .rows()
            .into_iter()
            .enumerate()
            .map(|(index, mut row)| {
                let cursor = if self.state.selected_idx == Some(index) {
                    '›'
                } else {
                    ' '
                };
                row.name = format!("{cursor} {}", row.name);
                row
            })
            .collect()
    }
}

impl<M: MenuModel> BottomPaneView for MenuView<M> {
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let control = key.modifiers == KeyModifiers::CONTROL;
        let plain = key.modifiers.is_empty();
        match key.code {
            KeyCode::Up => self.move_cursor(false),
            KeyCode::Char('k') if plain => self.move_cursor(false),
            KeyCode::Char('p') if control => self.move_cursor(false),
            KeyCode::Down => self.move_cursor(true),
            KeyCode::Char('j') if plain => self.move_cursor(true),
            KeyCode::Char('n') if control => self.move_cursor(true),
            KeyCode::Esc => self.complete = true,
            _ => {
                let selected = self.state.selected_idx;
                let before = self.model.rows().len();
                match self.model.on_key(key, selected, &self.app_event_tx) {
                    MenuKey::Close => self.complete = true,
                    MenuKey::Handled | MenuKey::Ignored => {}
                }
                // Other rows (a confirmation): from the first.
                let after = self.model.rows().len();
                if after != before {
                    self.state.selected_idx = None;
                }
                self.state.clamp_selection(after);
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

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        let unused = self.model.on_reply(reply);
        self.state.clamp_selection(self.model.rows().len());
        unused
    }
}

impl<M: MenuModel> Renderable for MenuView<M> {
    fn desired_height(&self, width: u16) -> u16 {
        let rows_width = width.saturating_sub(4);
        let rows = measure_rows_height(&self.rows(), &self.state, MAX_POPUP_ROWS, rows_width);
        // The surface's padding, the header, a gap, the rows, the hints.
        2 + self.header_lines().len() as u16 + 1 + rows + 1
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let [content_area, hints_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
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
                &self.rows(),
                &self.state,
                usize::from(rows_area.height),
                self.model.empty_message(),
            );
        }
        let hints_area = Rect {
            x: hints_area.x + 2,
            width: hints_area.width.saturating_sub(2),
            ..hints_area
        };
        Line::from(self.model.hints().dim()).render(hints_area, buf);
    }
}
