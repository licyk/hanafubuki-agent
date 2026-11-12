//! The pointer, as the settings center takes it (Codex's center has none):
//! the wheel moves the selection, a click on a tab shows that filter, a
//! click on a session or on a child in the details pane selects it, and a
//! click on the one selected opens it. A search, a rename, a confirmation
//! and the keys' help leave it be.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::{AgentsCenter, CenterAction, Mode};

/// The rows one notch of the wheel moves.
const WHEEL_ROWS: usize = 3;

impl AgentsCenter {
    pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) -> Option<CenterAction> {
        if self.mode != Mode::Browse {
            return None;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.wheel(false),
            MouseEventKind::ScrollDown => self.wheel(true),
            MouseEventKind::Down(MouseButton::Left) => return self.click(mouse.column, mouse.row),
            _ => {}
        }
        None
    }

    /// The selection moved by a notch, stopping at the ends.
    fn wheel(&mut self, down: bool) {
        let step = |index: usize, last: usize| {
            if down {
                (index + WHEEL_ROWS).min(last)
            } else {
                index.saturating_sub(WHEEL_ROWS)
            }
        };
        let rows = self.rows();
        if let Some(child) = self.child_selected {
            let Some(root) = rows.get(self.selected_index(&rows)) else {
                return;
            };
            let children = self.children_of(root.summary.session_id);
            if let Some(index) = children.iter().position(|item| item.session_id == child) {
                let next = step(index, children.len() - 1);
                self.child_selected = Some(children[next].session_id);
            }
            return;
        }
        if rows.is_empty() {
            return;
        }
        let next = step(self.selected_index(&rows), rows.len() - 1);
        self.selected = Some(rows[next].summary.session_id);
    }

    fn click(&mut self, column: u16, row: u16) -> Option<CenterAction> {
        let at = Position::new(column, row);
        let layout = &self.layout;
        if let Some((_, filter)) = layout.tabs.iter().find(|(cell, _)| cell.contains(at)) {
            let filter = *filter;
            self.set_filter(filter);
            return None;
        }
        if let Some((_, child)) = layout.children.iter().find(|(cell, _)| cell.contains(at)) {
            let child = *child;
            if self.child_selected == Some(child) {
                let summary = self.selected_child()?.clone();
                return Some(CenterAction::Open(Box::new(summary)));
            }
            self.child_selected = Some(child);
            return None;
        }
        let (_, session) = *layout.rows.iter().find(|(cell, _)| cell.contains(at))?;
        if self.selected == Some(session) && self.child_selected.is_none() {
            return self.open_selected();
        }
        self.selected = Some(session);
        self.child_selected = None;
        None
    }
}
