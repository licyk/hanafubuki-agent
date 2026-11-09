// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The panes in fullscreen (Codex's `app/side_pane.rs`, `app/pane_motion.rs`
//! and `app/workspace_hud.rs`): the main conversation on the left and the
//! side conversation docked on the right, a title row atop each and a
//! divider between them, which the pointer drags and the panes' keys
//! move; while the leader waits for its second key, a box lists them.
//! Narrower than [`SPLIT_MIN_WIDTH`], zoomed, or for a subagent shown, the
//! focused conversation fills the screen under one title row.

use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph, Widget};

use super::keys::{hint, hud_entries};
use super::{Focus, SecondKind, Workspace, WorkspaceEvent};
use crate::app::App;
use crate::app_event::AppEvent;
use crate::keymap::{self, Action};

/// The narrowest screen the panes share (Codex's).
pub(super) const SPLIT_MIN_WIDTH: u16 = 100;
/// The narrowest the side pane gets (Codex's).
const MIN_SIDE: u16 = 36;
/// The narrowest the main pane gets.
const MIN_MAIN: u16 = 40;
/// The HUD's width.
const HUD_WIDTH: u16 = 40;

/// Where the panes are drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneLayout {
    pub(crate) screen: Rect,
    /// The focused conversation's transcript and chat.
    pub(crate) focused: Rect,
    /// The other conversation's, when both are shown.
    pub(crate) passive: Option<Rect>,
    pub(crate) divider: Option<Rect>,
    pub(crate) titles: Vec<(Rect, Line<'static>)>,
    /// The leader's box.
    pub(crate) hud: Option<Rect>,
}

impl Workspace {
    /// The panes on `screen`; with no second conversation, the focused
    /// one fills it. `leader`: the leader waits for its second key, and a
    /// box lists the panes' keys.
    pub(crate) fn pane_layout(&self, screen: Rect, leader: bool) -> PaneLayout {
        let mut layout = PaneLayout {
            screen,
            focused: screen,
            passive: None,
            divider: None,
            titles: Vec::new(),
            hud: None,
        };
        let Some(second) = self.second.as_ref() else {
            return layout;
        };
        let side = matches!(second.kind, SecondKind::Side { .. });
        if side && !self.zoomed && screen.width >= SPLIT_MIN_WIDTH && screen.height > 2 {
            let side_width = self
                .side_width
                .unwrap_or(screen.width / 3)
                .clamp(MIN_SIDE, screen.width - MIN_MAIN - 1);
            let main_width = screen.width - side_width - 1;
            let main = Rect::new(screen.x, screen.y, main_width, screen.height);
            let side = Rect::new(main.right() + 1, screen.y, side_width, screen.height);
            layout.divider = Some(Rect::new(main.right(), screen.y, 1, screen.height));
            let (main_title, main_body) = split_title(main);
            let (side_title, side_body) = split_title(side);
            let main_focused = second.focus == Focus::Main;
            layout.titles = vec![
                (main_title, self.title(Focus::Main, main_focused, true)),
                (side_title, self.title(Focus::Second, !main_focused, true)),
            ];
            (layout.focused, layout.passive) = if main_focused {
                (main_body, Some(side_body))
            } else {
                (side_body, Some(main_body))
            };
        } else if screen.height > 1 {
            let (title, body) = split_title(screen);
            layout.titles = vec![(title, self.title(second.focus, true, false))];
            layout.focused = body;
        }
        if leader {
            let height = u16::try_from(hud_entries().len() + 2).unwrap_or(u16::MAX);
            let width = HUD_WIDTH.min(layout.focused.width);
            let x = layout.focused.right().saturating_sub(width);
            layout.hud =
                Some(Rect::new(x, layout.focused.y, width, height).intersection(layout.focused));
        }
        layout
    }

    /// The title of `which` conversation's pane, or (scrollback mode) the
    /// line that says the interface switched to it.
    pub(super) fn banner(&self, which: Focus) -> Line<'static> {
        self.title(which, true, false)
    }

    /// A pane's title; `split` when the panes share the screen, where the
    /// side pane is narrow.
    fn title(&self, which: Focus, focused: bool, split: bool) -> Line<'static> {
        let text = match (which, self.second.as_ref().map(|second| &second.kind)) {
            (Focus::Main, Some(SecondKind::Side { .. })) if focused => {
                format!("Main conversation · {} side", hint(Action::FocusPaneRight))
            }
            (Focus::Main, _) => "Main conversation".to_string(),
            (Focus::Second, Some(SecondKind::Side { .. })) if focused && split => {
                "Side · Esc returns · /side close".to_string()
            }
            (Focus::Second, Some(SecondKind::Side { .. })) if focused => {
                "Side conversation · Esc returns · /side close closes it".to_string()
            }
            (Focus::Second, Some(SecondKind::Side { .. })) => {
                format!("Side conversation · {}", hint(Action::FocusPaneRight))
            }
            (
                Focus::Second,
                Some(SecondKind::Child {
                    title,
                    parent_title,
                    ..
                }),
            ) => {
                let back = hint(Action::FocusPaneLeft);
                format!("Subagent {title} · from {parent_title} · Esc or {back} returns")
            }
            (Focus::Second, None) => String::new(),
        };
        if focused {
            Line::from(format!(" {text} ").bold().cyan())
        } else {
            Line::from(format!(" {text} ").dim())
        }
    }

    /// Moves the divider `delta` columns to the left (the side pane grows).
    pub(super) fn move_divider(&mut self, delta: i32) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let (Some(divider), Some(_)) = (layout.divider, layout.passive) else {
            return;
        };
        let current = i32::from(layout.screen.right() - divider.right());
        self.side_width = u16::try_from((current + delta).max(1)).ok();
    }

    /// The panes as drawn, for the pointer and the divider's keys.
    pub(crate) fn remember_layout(&mut self, layout: PaneLayout) {
        self.layout = Some(layout);
    }
}

/// A pane's title row, and the rest.
fn split_title(area: Rect) -> (Rect, Rect) {
    let title = Rect { height: 1, ..area };
    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    (title, body)
}

/// The titles, the divider and the leader's box over the panes.
pub(crate) fn paint_chrome(buf: &mut Buffer, layout: &PaneLayout, dragging: bool) {
    for (area, title) in &layout.titles {
        Clear.render(*area, buf);
        title.clone().render(*area, buf);
    }
    if let Some(divider) = layout.divider {
        let style = if dragging {
            Style::new().cyan()
        } else {
            Style::new().dim()
        };
        for y in divider.top()..divider.bottom() {
            buf[(divider.x, y)].set_symbol("│").set_style(style);
        }
    }
    if let Some(hud) = layout.hud {
        Clear.render(hud, buf);
        let lines: Vec<Line<'static>> = hud_entries()
            .into_iter()
            .map(|(keys, label)| Line::from(vec![format!(" {keys:<6} ").cyan(), label.into()]))
            .collect();
        Paragraph::new(lines)
            .block(
                Block::bordered().title(format!(" {} ", keymap::current().label(Action::Leader))),
            )
            .render(hud, buf);
    }
}

impl App {
    /// The pointer on the panes: the divider dragged, a click in the other
    /// pane focusing it. `true` when it was theirs.
    pub(crate) fn handle_workspace_mouse(&mut self, mouse: MouseEvent) -> bool {
        let workspace = &mut self.workspace;
        let Some(layout) = workspace.layout.clone() else {
            return false;
        };
        let at = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left)
                if layout.divider.is_some_and(|divider| divider.contains(at)) =>
            {
                workspace.dragging = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if workspace.dragging => {
                let width = layout.screen.right().saturating_sub(mouse.column + 1);
                workspace.side_width = Some(width);
            }
            MouseEventKind::Up(MouseButton::Left) if workspace.dragging => {
                workspace.dragging = false;
            }
            MouseEventKind::Down(MouseButton::Left)
                if layout.passive.is_some_and(|passive| passive.contains(at)) =>
            {
                self.app_event_tx
                    .send(AppEvent::Workspace(WorkspaceEvent::FocusOther));
            }
            _ => return false,
        }
        self.schedule_frame(Instant::now());
        true
    }
}
