//! Scrollback's tail (the plan's §4.11): the newest entries not yet written
//! into the terminal's scrollback, painted in the inline viewport above the
//! chat as Fullscreen lays them out, with their controls. The tail does not
//! scroll: it is painted from its first row or, when taller than its area,
//! as its last rows under the header of the entry they start in. Only the
//! disclosure's keys (F4) and a click on a control or a header reach it,
//! and it stays at its latest rows whatever they do.
//!
//! The app decides what leaves the tail; this gives the lines it writes: an
//! entry's rows as shown, its control replaced by the transcript key's hint
//! while collapsed and left out once expanded.

use ratatui::crossterm::event::KeyEvent;
use ratatui::style::Stylize;
use ratatui::text::Span;

use super::*;
use crate::keymap::{self, Action};

/// One entry of the tail as laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TailEntry {
    pub(crate) rows: usize,
    /// It has a control that shows or hides its details.
    pub(crate) control: bool,
}

impl TranscriptView {
    /// Lays the tail's entries out at `width`, the first `separated` from
    /// the lines written above it. Called once a frame, before the tail is
    /// written from or painted.
    pub(crate) fn measure_tail(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        width: u16,
        separated: bool,
    ) -> Vec<TailEntry> {
        self.cache.begin_frame();
        self.scrollback_tail = true;
        self.prepare_width(width);
        self.leading_separator = separated;
        (0..cells.len())
            .filter_map(|index| self.current_layout(cells, index))
            .map(|layout| TailEntry {
                rows: layout.row_count(),
                control: layout.disclosure,
            })
            .collect()
    }

    /// The rows of the tail's entry `index` as Scrollback writes them.
    pub(crate) fn tail_lines(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        index: usize,
        separated: bool,
    ) -> Vec<Line<'static>> {
        self.scrollback_tail = true;
        self.leading_separator = separated;
        let Some(layout) = self.current_layout(cells, index) else {
            return Vec::new();
        };
        let control = layout.disclosure_row();
        let expanded = cells
            .get(index)
            .is_some_and(|cell| self.disclosure.is_expanded(&cell.activity_ids()));
        (0..layout.row_count())
            .filter_map(|row| {
                if Some(row) != control {
                    Some(layout.row_line(row))
                } else {
                    (!expanded).then(|| expand_hint(layout.disclosure_columns().start))
                }
            })
            .collect()
    }

    /// Paints the tail laid out by [`Self::measure_tail`] in `area`.
    pub(crate) fn render_tail(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        cells: &[Arc<dyn HistoryCell>],
    ) {
        Clear.render(area, buf);
        self.area = area;
        self.visible.clear();
        let mut rows = Vec::new();
        for index in 0..cells.len() {
            let count = self
                .current_layout(cells, index)
                .map_or(0, |layout| layout.row_count());
            rows.extend((0..count).map(|row| (index, row)));
        }
        let height = usize::from(area.height);
        if rows.len() > height {
            rows.drain(..rows.len() - height);
            if let Some((index, row)) = rows.first_mut() {
                let header = self
                    .current_layout(cells, *index)
                    .and_then(|layout| layout.header_row())
                    .unwrap_or(0);
                *row = (*row).min(header);
            }
        }
        for ((index, row), y) in rows.into_iter().zip(area.top()..) {
            let Some(layout) = self.current_layout(cells, index) else {
                continue;
            };
            let activity_ids = self.displayed_activity_ids(cells, index);
            let row_area = Rect::new(area.x, y, area.width, 1);
            layout.render(row_area, buf, row);
            self.render_disclosure(&activity_ids, &layout, row, row_area, buf);
            self.visible.push(VisibleRow {
                index,
                row,
                layout,
                key: self.entry_key(cells, index),
                activity_ids,
            });
        }
    }

    /// A left press at a cell of the screen: `true` when it toggled the
    /// control painted there. On a header, the release decides.
    pub(crate) fn press_tail(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        column: u16,
        row: u16,
    ) -> bool {
        self.cancel_header_press();
        let toggled = self.toggle_disclosure_at(cells, column, row);
        if !toggled {
            self.press_header_at(column, row);
        }
        self.position = Position::Latest;
        toggled
    }

    /// A left release: `true` when it ended a click on a header, which
    /// toggled its entry.
    pub(crate) fn release_tail(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        column: u16,
        row: u16,
    ) -> bool {
        let toggled = self.release_header_at(cells, column, row);
        self.position = Position::Latest;
        toggled
    }

    /// F4 and, while an activity is focused, the keys that move between the
    /// activities and show or hide one; `None` for a key it does not take.
    pub(crate) fn handle_tail_key(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        let action = self.handle_disclosure_key(key, cells);
        self.position = Position::Latest;
        action
    }
}

/// What a written entry with more to show ends with, at its control's
/// indent.
fn expand_hint(indent: u16) -> Line<'static> {
    let mut spans = vec![Span::from(" ".repeat(usize::from(indent)))];
    match keymap::hint(Action::OpenTranscript) {
        Some(key) => {
            spans.push(key.into());
            spans.push(" to expand".into());
        }
        None => spans.push("expand in the transcript".into()),
    }
    Line::from(spans).dim()
}

#[cfg(test)]
mod tests;
