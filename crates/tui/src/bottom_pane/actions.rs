use std::cell::RefCell;

use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    style::Stylize,
    text::Line,
    widgets::Widget,
};

#[derive(Default)]
pub(crate) struct ActionButtons(RefCell<Vec<(Rect, KeyCode)>>);

impl ActionButtons {
    pub(crate) fn render(&self, area: Rect, buffer: &mut Buffer, buttons: &[(&str, KeyCode)]) {
        let mut hits = self.0.borrow_mut();
        hits.clear();
        let mut x = area.x;
        for &(label, key) in buttons {
            let line = Line::from(label.dim());
            let rect = Rect::new(x, area.y, line.width() as u16, 1).intersection(area);
            line.render(rect, buffer);
            hits.push((rect, key));
            x = rect.right().saturating_add(2);
        }
    }

    pub(crate) fn key(&self, mouse: MouseEvent) -> Option<KeyCode> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        self.0
            .borrow()
            .iter()
            .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
            .map(|(_, key)| *key)
    }
}
