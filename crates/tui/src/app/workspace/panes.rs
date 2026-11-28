// Modified from OpenAI Codex 57a808a392 (Apache-2.0).
// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! The panes in fullscreen (Codex's `app/side_pane.rs`, `app/pane_motion.rs`
//! and `app/workspace_hud.rs`): the selected agent on the left and the
//! side conversation docked on the right, a single row for its tabs and a
//! divider between them, which the pointer drags and the panes' keys
//! move; while the leader waits for its second key, a box lists them.
//! Narrower than [`SPLIT_MIN_WIDTH`], zoomed, or without a side pane, the
//! focused pane fills the screen.

#[cfg(test)]
mod tests;

use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph, Widget};

use super::keys::{hint, hud_entries};
use super::{Focus, Workspace, WorkspaceEvent};
use crate::app::App;
use crate::app_event::AppEvent;
use crate::keymap::{self, Action};

/// The narrowest screen the panes share (Codex's).
pub(in crate::app) const SPLIT_MIN_WIDTH: u16 = 100;
/// The narrowest the side pane gets (Codex's).
const MIN_SIDE: u16 = 36;
/// The narrowest the main pane gets.
const MIN_MAIN: u16 = 64;
/// The HUD's width.
const HUD_WIDTH: u16 = 40;

fn clamp_width(total: u16, width: i32) -> u16 {
    width.clamp(
        i32::from(MIN_SIDE),
        i32::from(total.saturating_sub(MIN_MAIN + 1).max(MIN_SIDE)),
    ) as u16
}

/// Where the panes are drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneLayout {
    pub(crate) screen: Rect,
    /// The focused conversation's transcript and chat.
    pub(crate) focused: Rect,
    /// The other conversation's, when both are shown.
    pub(crate) passive: Option<Rect>,
    pub(crate) divider: Option<Rect>,
    pub(crate) panel: Option<Rect>,
    pub(crate) strip: Option<Rect>,
    pub(crate) titles: Vec<(Rect, Line<'static>)>,
    /// The leader's box.
    pub(crate) hud: Option<Rect>,
}

impl Workspace {
    pub(super) fn split_panes(&self, screen: Rect) -> Option<(Rect, Rect, Rect)> {
        self.split_panes_at(screen, 1.0)
    }

    /// Whether a side pane should be shown: a docked or panel sidebar, or a
    /// second conversation. The slide follows this, so the pane it draws
    /// outlasts the setting turning off until the animation ends.
    pub(in crate::app) fn side_pane_open(&self) -> bool {
        self.sidebar.uses_fullscreen() || self.second.is_some()
    }

    /// [`Self::split_panes`] with `fraction` of the side pane's width drawn:
    /// the side pane slides open and shut, so a frame part-way through draws
    /// that share of it, and nothing once it reaches the main pane's edge.
    pub(super) fn split_panes_at(&self, screen: Rect, fraction: f64) -> Option<(Rect, Rect, Rect)> {
        if self.zoomed || screen.width < SPLIT_MIN_WIDTH || screen.height == 0 || fraction <= 0.0 {
            return None;
        }
        let preferred = match self.side_width {
            Some((total, width)) if screen.width == total => width,
            Some((total, width)) => {
                let total = u32::from(total);
                let percent = ((u32::from(width) * 100 + total / 2) / total).clamp(1, 99);
                ((u32::from(screen.width) * percent + 50) / 100) as u16
            }
            None => screen.width / 3,
        };
        let full = clamp_width(screen.width, i32::from(preferred));
        let width = (f64::from(full) * fraction.min(1.0)).round().max(1.0) as u16;
        let width = width.min(full).min(screen.width.saturating_sub(1));
        let side = Rect::new(screen.right() - width, screen.y, width, screen.height);
        let main = Rect {
            width: screen.width - width - 1,
            ..screen
        };
        Some((
            main,
            side,
            Rect::new(main.right(), screen.y, 1, screen.height),
        ))
    }

    /// The panes on `screen`; with no second conversation, the focused
    /// one fills it. `leader`: the leader waits for its second key, and a
    /// box lists the panes' keys.
    /// The panes on `screen`; `fraction` is how much of the side pane the
    /// slide has drawn (1: all of it).
    pub(crate) fn pane_layout(&self, screen: Rect, leader: bool, fraction: f64) -> PaneLayout {
        let mut layout = PaneLayout {
            screen,
            focused: screen,
            passive: None,
            divider: None,
            panel: None,
            strip: None,
            titles: Vec::new(),
            hud: None,
        };
        if self.sidebar.showing_panel() {
            self.sidebar_layout(&mut layout);
            return layout;
        }
        let Some(second) = self.second.as_ref() else {
            if self.child.is_some() && screen.height > 1 {
                let (title, body) = split_title(screen);
                layout.titles = vec![(title, self.banner(Focus::Main))];
                layout.focused = body;
            }
            return layout;
        };
        if let Some((main, side, divider)) = self.split_panes_at(screen, fraction) {
            layout.divider = Some(divider);
            (layout.focused, layout.passive) = if second.focus == Focus::Main {
                (main, Some(side))
            } else {
                (side, Some(main))
            };
        }
        if let Some(side) = if second.focus == Focus::Second {
            Some(&mut layout.focused)
        } else {
            layout.passive.as_mut()
        } {
            let (strip, body) = split_title(*side);
            layout.strip = Some(strip);
            *side = body;
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
        let text = match which {
            Focus::Main if self.child.is_some() => {
                let child = self.child.as_ref().unwrap();
                let back = hint(Action::FocusPaneLeft);
                format!(
                    "Subagent {} · from {} · Esc or {back} returns",
                    child.title, child.parent_title
                )
            }
            Focus::Main if self.second.is_some() => {
                format!("Main conversation · {} side", hint(Action::FocusPaneRight))
            }
            Focus::Main => "Main conversation".to_string(),
            Focus::Second
                if self
                    .second
                    .as_ref()
                    .is_some_and(|tab| tab.kind == super::side_tabs::TabKind::Agent) =>
            {
                "Subagent · read-only".to_owned()
            }
            Focus::Second if self.sidebar.uses_fullscreen() => "Side conversation".to_owned(),
            Focus::Second => format!(
                "Side conversation · {} or /close closes · Esc returns",
                hint(Action::HidePane)
            ),
        };
        Line::from(format!(" {text} ").bold().cyan())
    }

    pub(super) fn set_side_width(&mut self, total: u16, width: i32) {
        if total >= SPLIT_MIN_WIDTH {
            self.side_width = Some((total, clamp_width(total, width)));
        }
    }

    /// Moves the divider `delta` columns to the left (the side pane grows).
    pub(super) fn move_divider(&mut self, delta: i32) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let Some(divider) = layout.divider else {
            return;
        };
        let current = i32::from(layout.screen.right() - divider.right());
        self.set_side_width(layout.screen.width, current + delta);
    }

    /// The panes as drawn, for the pointer and the divider's keys.
    pub(crate) fn remember_layout(&mut self, layout: PaneLayout) {
        self.layout = Some(layout);
    }
}

/// A pane's title row, and the rest.
pub(super) fn split_title(area: Rect) -> (Rect, Rect) {
    let title = Rect {
        height: area.height.min(1),
        ..area
    };
    let body = Rect {
        y: area.y.saturating_add(1),
        height: area.height.saturating_sub(1),
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
                workspace.set_side_width(layout.screen.width, i32::from(width));
            }
            MouseEventKind::Up(MouseButton::Left) if workspace.dragging => {
                workspace.dragging = false;
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                if layout.passive.is_some_and(|passive| passive.contains(at)) =>
            {
                if let Some(parked) = workspace.parked_mut() {
                    parked
                        .transcript_view
                        .handle_mouse(mouse, &parked.transcript_cells);
                }
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
