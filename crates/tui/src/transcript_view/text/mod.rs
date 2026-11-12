// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A cell's lines as one text with source positions, wrapped into display
//! rows, shared by painting, selection and search.
//!
//! Each input line is a logical line: lines are joined with `\n` into
//! `text`, and a line wider than the view wraps at its words.
//! Codex keeps wrapping provenance on its lines, so a wrapped paragraph is
//! one logical line again; the cells here wrap their own lines, so text
//! copied across a wrap the cell made carries a line break. Display rows
//! added for spacing and disclosure controls never enter `text`. Tabs are
//! expanded to spaces, so the copied text has the columns shown.

mod wrap;

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use wrap::{expand_tabs, wrap_line};

/// One cell's text and its width-dependent display rows.
pub(crate) struct TextLayout {
    lines: Vec<Line<'static>>,
    text: String,
    rows: Vec<TextRow>,
    width: u16,
    pub(super) separated: bool,
    pub(super) disclosure: bool,
    leading_blank_hidden: bool,
    disclosure_control: Option<DisclosureControl>,
}

struct DisclosureControl {
    label: String,
    source_offset: usize,
    row: usize,
}

struct TextRow {
    line: Line<'static>,
    source: Range<usize>,
    /// The source offset of the logical line's end; absent on wrapped and
    /// synthetic rows.
    line_end: Option<usize>,
    /// The column the row's source starts at.
    first_column: usize,
}

impl TextLayout {
    pub(crate) fn new(lines: Vec<Line<'static>>, width: u16) -> Self {
        Self::from_lines(lines.into_iter().map(expand_tabs).collect(), width)
    }

    /// The same content laid out at another width, with its spacing and
    /// disclosure control.
    pub(super) fn rewrap(&self, width: u16) -> Self {
        let mut layout = Self::from_lines(self.lines.clone(), width);
        if let Some(control) = &self.disclosure_control {
            layout =
                layout.with_disclosure_control_at(control.label.clone(), control.source_offset);
        }
        if self.leading_blank_hidden {
            layout = layout.without_leading_blank_row();
        }
        if self.separated {
            layout.with_leading_separator()
        } else {
            layout
        }
    }

    /// Appends a control row outside the copied and searched text.
    pub(super) fn with_disclosure_control(self, label: String) -> Self {
        let source_offset = self.text.len();
        self.with_disclosure_control_at(label, source_offset)
    }

    /// Places a control after an activity, before any text that follows it.
    pub(super) fn with_disclosure_control_at(
        mut self,
        label: String,
        source_offset: usize,
    ) -> Self {
        if self.text.is_empty() || label.is_empty() {
            return self;
        }
        if let Some(control) = self.disclosure_control.take() {
            self.rows.remove(control.row);
        }
        let source_offset = self
            .text
            .floor_char_boundary(source_offset.min(self.text.len()));
        let control_row = self
            .rows
            .iter()
            .rposition(|row| row.source.end <= source_offset)
            .map_or(0, |row| row + 1);
        let indent = usize::from(self.width / 4).min(4);
        let label_line = Line::from(label.clone()).dim();
        let mut row = TextRow {
            line: label_line,
            source: source_offset..source_offset,
            line_end: None,
            first_column: indent,
        };
        clip_row(&mut row, usize::from(self.width));
        self.rows.insert(control_row, row);
        self.disclosure = true;
        self.disclosure_control = Some(DisclosureControl {
            label,
            source_offset,
            row: control_row,
        });
        self
    }

    /// Adds a blank row between entries without moving any source position.
    pub(super) fn with_leading_separator(mut self) -> Self {
        if !self.separated && !self.rows.is_empty() {
            self.rows.insert(
                0,
                TextRow {
                    line: Line::default(),
                    source: 0..0,
                    line_end: None,
                    first_column: 0,
                },
            );
            if let Some(control) = &mut self.disclosure_control {
                control.row += 1;
            }
            self.separated = true;
        }
        self
    }

    /// Hides one leading blank row without moving any source position.
    pub(super) fn without_leading_blank_row(mut self) -> Self {
        if self.rows.first().is_some_and(|row| row.line.width() == 0) {
            self.rows.remove(0);
            self.leading_blank_hidden = true;
            if let Some(control) = &mut self.disclosure_control {
                control.row = control.row.saturating_sub(1);
            }
        }
        self
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// A display row as a line of its own, its indent included.
    pub(super) fn row_line(&self, row: usize) -> Line<'static> {
        let Some(row) = self.rows.get(row) else {
            return Line::default();
        };
        let mut line = row.line.clone();
        if row.first_column > 0 {
            line.spans.insert(0, " ".repeat(row.first_column).into());
        }
        line
    }

    /// The entry's first row with text: its header, past a separator and a
    /// prompt's blank padding.
    pub(super) fn header_row(&self) -> Option<usize> {
        self.rows.iter().position(|row| !row.source.is_empty())
    }

    pub(super) fn disclosure_row(&self) -> Option<usize> {
        self.disclosure_control.as_ref().map(|control| control.row)
    }

    pub(super) fn disclosure_columns(&self) -> Range<u16> {
        let Some(row) = self.disclosure_row().and_then(|row| self.rows.get(row)) else {
            return 0..0;
        };
        let end = (row.first_column + row.line.width()).min(usize::from(self.width));
        row.first_column as u16..end as u16
    }

    /// Paints the visible rows, with each line's style over its whole row.
    pub(crate) fn render(&self, area: Rect, buf: &mut Buffer, start_row: usize) {
        for (screen_row, row) in self.visible_rows(area, start_row) {
            let y = area.y + screen_row;
            buf.set_style(Rect::new(area.x, y, area.width, 1), row.line.style);
            let x = area.x + row.first_column as u16;
            let width = area.width.saturating_sub(row.first_column as u16);
            buf.set_line(x, y, &row.line, width);
        }
    }

    /// The source position under a cell, clamped to the row's text.
    pub(super) fn position_at(&self, row: usize, column: u16) -> usize {
        let Some(row) = self.rows.get(row) else {
            return self.text.len();
        };
        let mut current_column = row.first_column;
        for (offset, grapheme) in self.text[row.source.clone()].grapheme_indices(true) {
            current_column += grapheme.width();
            if usize::from(column) < current_column {
                return row.source.start + offset;
            }
        }
        row.source.end
    }

    /// A keyboard caret's position: a grapheme start, except on a truly
    /// empty row.
    pub(super) fn cursor_position_at(&self, row: usize, column: u16) -> usize {
        let offset = self.position_at(row, column);
        let Some(row) = self.rows.get(row) else {
            return offset;
        };
        if row.source.start == row.source.end || offset < row.source.end {
            return offset;
        }
        self.text[row.source.clone()]
            .grapheme_indices(true)
            .next_back()
            .map_or(row.source.start, |(offset, _)| row.source.start + offset)
    }

    /// The first selectable grapheme on a display row; synthetic rows have
    /// none.
    pub(super) fn first_cursor_position(&self, row: usize) -> Option<usize> {
        let row = self.rows.get(row)?;
        self.text[row.source.clone()]
            .grapheme_indices(true)
            .next()
            .map(|(offset, _)| row.source.start + offset)
    }

    /// Whether a source grapheme is painted on some display row.
    pub(super) fn is_cursor_position(&self, offset: usize) -> bool {
        self.rows.iter().any(|row| row.source.contains(&offset))
    }

    /// The offset after the grapheme at `offset` on the caret's row; on an
    /// empty row the caret stays.
    pub(super) fn after_cursor_grapheme(&self, row: usize, offset: usize) -> usize {
        let Some(row) = self.rows.get(row) else {
            return offset;
        };
        if row.source.start == row.source.end {
            return offset;
        }
        let offset = self
            .text
            .floor_char_boundary(offset.clamp(row.source.start, row.source.end));
        offset
            + self.text[offset..row.source.end]
                .graphemes(true)
                .next()
                .map_or(0, str::len)
    }

    /// The display row holding a source position. A disclosure control
    /// shares its offset with the text after it, which keeps its rows.
    pub(super) fn row_for_offset(&self, offset: usize) -> usize {
        let before_control = match self.disclosure_row() {
            Some(control_row) => {
                let after_control = &self.rows[control_row + 1..];
                let matching_rows = after_control.partition_point(|row| row.source.start <= offset);
                if matching_rows > 0 {
                    return control_row + matching_rows;
                }
                control_row
            }
            None => self.rows.len(),
        };
        self.rows[..before_control]
            .partition_point(|row| row.source.start <= offset)
            .saturating_sub(1)
    }

    pub(super) fn column_for_offset(&self, offset: usize) -> u16 {
        self.column_for_offset_on_row(self.row_for_offset(offset), offset)
    }

    /// A source position's column on a given row, where rows share offsets.
    pub(super) fn column_for_offset_on_row(&self, row: usize, offset: usize) -> u16 {
        let Some(row) = self.rows.get(row) else {
            return 0;
        };
        let offset = self
            .text
            .floor_char_boundary(offset.clamp(row.source.start, row.source.end));
        let column = row.first_column + self.text[row.source.start..offset].width();
        column.min(usize::from(self.width)) as u16
    }

    /// Reverses the source graphemes in `range`, leaving padding as it is.
    pub(super) fn highlight(
        &self,
        range: Range<usize>,
        area: Rect,
        buf: &mut Buffer,
        start_row: usize,
    ) {
        for (screen_row, row) in self.visible_rows(area, start_row) {
            let mut column = row.first_column;
            for (offset, grapheme) in self.text[row.source.clone()].grapheme_indices(true) {
                let start = row.source.start + offset;
                let width = grapheme.width();
                if start < range.end && start + grapheme.len() > range.start {
                    for selected in column..(column + width).min(usize::from(area.width)) {
                        buf[(area.x + selected as u16, area.y + screen_row)]
                            .set_style(Style::default().add_modifier(Modifier::REVERSED));
                    }
                }
                column += width;
            }
        }
    }

    /// Highlights `range`, and one cell after each selected line break, on
    /// a non-empty row; `continues` says the selection goes on into the
    /// next entry, past this one's end.
    pub(super) fn highlight_selection(
        &self,
        range: Range<usize>,
        continues: bool,
        area: Rect,
        buf: &mut Buffer,
        start_row: usize,
    ) {
        self.highlight(range.clone(), area, buf, start_row);
        for (screen_row, row) in self.visible_rows(area, start_row) {
            if row.source.is_empty() {
                continue;
            }
            let Some(end) = row.line_end else { continue };
            let selected = range.contains(&end) || (end == self.text.len() && continues);
            let column = row.first_column + self.text[row.source.clone()].width();
            if selected && column < usize::from(self.width) {
                buf[(area.x + column as u16, area.y + screen_row)]
                    .set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }
    }

    fn visible_rows(&self, area: Rect, start_row: usize) -> impl Iterator<Item = (u16, &TextRow)> {
        let height = if area.width == 0 { 0 } else { area.height };
        self.rows
            .iter()
            .skip(start_row)
            .take(usize::from(height))
            .enumerate()
            .map(|(row, text)| (row as u16, text))
    }

    fn from_lines(lines: Vec<Line<'static>>, width: u16) -> Self {
        let width = width.max(1);
        let mut text = String::new();
        let mut rows = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            let start = text.len();
            text.extend(line.spans.iter().map(|span| span.content.as_ref()));
            rows.extend(layout_line(line, start, width));
            if let Some(row) = rows.last_mut() {
                row.line_end = Some(text.len());
            }
        }
        Self {
            lines,
            text,
            rows,
            width,
            separated: false,
            disclosure: false,
            leading_blank_hidden: false,
            disclosure_control: None,
        }
    }
}

/// The display rows of one logical line, its source starting at
/// `text_start`.
fn layout_line(line: &Line<'static>, text_start: usize, width: u16) -> Vec<TextRow> {
    let columns = usize::from(width);
    wrap_line(line, columns)
        .into_iter()
        .map(|(source, row_line)| {
            let text_width = row_line.width();
            let first_column = match line.alignment.unwrap_or(Alignment::Left) {
                Alignment::Left => 0,
                Alignment::Center => columns.saturating_sub(text_width) / 2,
                Alignment::Right => columns.saturating_sub(text_width),
            };
            TextRow {
                line: row_line,
                source: text_start + source.start..text_start + source.end,
                line_end: None,
                first_column,
            }
        })
        .collect()
}

/// Cuts a synthetic row to the columns left after its indent.
fn clip_row(row: &mut TextRow, width: usize) {
    let available = width.saturating_sub(row.first_column);
    if row.line.width() <= available {
        return;
    }
    let text: String = row
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let mut end = 0;
    let mut used = 0;
    for (offset, grapheme) in text.grapheme_indices(true) {
        if used + grapheme.width() > available {
            break;
        }
        used += grapheme.width();
        end = offset + grapheme.len();
    }
    row.line = wrap::slice_line(&row.line, 0..end);
}

#[cfg(test)]
mod tests;
