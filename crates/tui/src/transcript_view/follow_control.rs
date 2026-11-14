// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The return-to-latest control a caller places in a row of its own (the
//! owned transcript, in the gap above the composer). It is hidden while
//! the transcript's last row is visible; rendering sets its hit area, so a
//! hidden control never takes the pointer.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position as ScreenPosition;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::*;

#[derive(Default)]
pub(super) struct FollowControl {
    area: Option<Rect>,
    pointer: Option<ScreenPosition>,
    pressed: bool,
}

impl TranscriptView {
    pub(crate) fn render_follow_control(&mut self, area: Option<Rect>, buf: &mut Buffer) {
        let Some(area) = area.filter(|area| !area.is_empty()) else {
            self.follow_control = FollowControl::default();
            return;
        };
        let Some(label) = self.follow_control_label(area.width) else {
            self.follow_control = FollowControl::default();
            return;
        };
        let width = label.width() as u16;
        let target = Rect::new(area.x + (area.width - width) / 2, area.y, width, 1);
        let hovered = self
            .follow_control
            .pointer
            .is_some_and(|point| target.contains(point));
        self.follow_control.area = Some(target);
        let style = crate::style::user_message_style().fg(crate::style::accent_color());
        let style = if hovered {
            style.reversed().bold()
        } else {
            style
        };
        Paragraph::new(label).style(style).render(target, buf);
    }

    pub(super) fn follow_control_label(&self, width: u16) -> Option<&'static str> {
        if self.area.is_empty()
            || self.tail_visible
            || self.selection.is_some()
            || self.is_search_editing()
            || self.highlight.is_some()
        {
            return None;
        }
        let labels: &[&str] = if self.unseen_activity {
            &[
                " New activity · ↓ Back to bottom ",
                " New activity · ↓ Bottom ",
                " New · ↓ Bottom ",
                " ↓ Bottom ",
                " ↓ ",
            ]
        } else {
            &[" ↓ Back to bottom ", " ↓ Bottom ", " ↓ "]
        };
        labels
            .iter()
            .copied()
            .find(|label| label.width() <= usize::from(width))
    }

    pub(super) fn handle_follow_control_mouse(&mut self, event: MouseEvent) -> Option<ViewAction> {
        let control = &mut self.follow_control;
        let area = control.area?;
        let point = ScreenPosition::new(event.column, event.row);
        let was_hovered = control.pointer.is_some_and(|point| area.contains(point));
        let inside = area.contains(point);
        control.pointer = Some(point);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if inside => {
                control.pressed = true;
                Some(ViewAction::Changed)
            }
            MouseEventKind::Up(MouseButton::Left) if control.pressed => {
                control.pressed = false;
                if inside {
                    self.jump_to_latest();
                }
                Some(ViewAction::Changed)
            }
            MouseEventKind::Drag(MouseButton::Left) if control.pressed => Some(ViewAction::Changed),
            MouseEventKind::Moved if inside != was_hovered => Some(ViewAction::Changed),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
