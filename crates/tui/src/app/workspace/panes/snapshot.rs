//! The last picture a side pane showed, kept while it slides out: a closed
//! pane's conversation is torn down the moment it closes, so the slide draws
//! what was there instead of the pane itself.

//! The last picture a side pane showed, kept while it slides out: a closed
//! pane's conversation is torn down the moment it closes, so the slide draws
//! what was there instead of the pane itself.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;

/// Cells of a pane as it last stood, with the rectangle they filled.
pub(in crate::app) struct PaneSnapshot {
    area: Rect,
    cells: Vec<Cell>,
}

impl PaneSnapshot {
    /// `area` of `buffer` as it is.
    pub(in crate::app) fn take(area: Rect, buffer: &Buffer) -> Self {
        let mut cells = Vec::with_capacity(usize::from(area.width) * usize::from(area.height));
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                cells.push(buffer[(x, y)].clone());
            }
        }
        Self { area, cells }
    }

    /// The picture drawn into `target`, as much of it as `target` has room
    /// for from its left edge: the pane narrowed, its content with it.
    pub(in crate::app) fn paint(&self, target: Rect, buffer: &mut Buffer) {
        if self.area.width == 0 || self.area.height == 0 {
            return;
        }
        let width = target.width.min(self.area.width);
        for (row, y) in (target.top()..target.bottom()).enumerate() {
            if row >= usize::from(self.area.height) {
                break;
            }
            for column in 0..usize::from(width) {
                let source = row * usize::from(self.area.width) + column;
                let Some(cell) = self.cells.get(source) else {
                    break;
                };
                if let Some(destination) = buffer.cell_mut((target.x + column as u16, y)) {
                    *destination = cell.clone();
                }
            }
        }
    }

    /// The pane as it stood; `None` before anything was drawn.
    #[cfg(test)]
    pub(in crate::app) fn area(&self) -> Rect {
        self.area
    }
}

#[cfg(test)]
mod tests;
