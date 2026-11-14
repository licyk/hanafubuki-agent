// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Pipe tables. The writer collects a table's header and body rows into a
//! [`TableState`]; at its end they are laid out:
//!
//! 1. Spillover: a row the parser made of the prose after the table (no
//!    pipe, only its first cell) is taken out, to follow the table as text.
//! 2. Every row gets as many cells as there are columns.
//! 3. Column widths are allocated (`layout`): each column starts at its
//!    content's width and the columns shrink by priority until they fit.
//! 4. The grid, header and rows apart by rules, while its values still
//!    read; otherwise each body row becomes a record of `header  value`
//!    fields (`key_value`).
//! 5. The spillover rows follow, wrapped as prose.

mod key_value;
mod layout;
mod spillover;

use std::ops::Range;

use pulldown_cmark::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::output::spans_display_width;
use super::Writer;
use crate::terminal_hyperlinks::{remap_source_wrapped_line, HyperlinkLine};
use crate::wrapping::{word_wrap_line_with_source, RtOptions};
use layout::{collect_table_column_metrics, compute_column_widths, line_display_width};

const TABLE_COLUMN_GAP: usize = 2;
const TABLE_CELL_PADDING: usize = 1;
const TABLE_HEADER_SEPARATOR_CHAR: char = '━';
const TABLE_BODY_SEPARATOR_CHAR: char = '─';

/// One cell's content: a line per hard break, with its inline styles and
/// links.
#[derive(Clone, Debug, Default)]
struct TableCell {
    lines: Vec<HyperlinkLine>,
}

impl TableCell {
    fn ensure_line(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(HyperlinkLine::default());
        }
    }

    fn push(&mut self, part: HyperlinkLine) {
        self.ensure_line();
        if let Some(line) = self.lines.last_mut() {
            line.append(part);
        }
    }

    fn hard_break(&mut self) {
        self.lines.push(HyperlinkLine::default());
    }

    /// The text, its lines joined by spaces: what widths are measured on.
    fn plain_text(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                line.line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug)]
struct TableBodyRow {
    cells: Vec<TableCell>,
    /// Whether its source line starts or ends with a pipe.
    has_table_pipe_syntax: bool,
}

/// A table being read, from `Tag::Table` to its end; the cell being read
/// takes the inline events.
#[derive(Debug)]
pub(super) struct TableState {
    alignments: Vec<Alignment>,
    header: Option<Vec<TableCell>>,
    rows: Vec<TableBodyRow>,
    current_row: Option<Vec<TableCell>>,
    current_row_has_table_pipe_syntax: bool,
    /// Where the current row's text ends, its newline left out.
    current_row_end: usize,
    current_cell: Option<TableCell>,
    in_header: bool,
}

/// A table's rows: the grid's or the records', laid out to the width
/// already unless they are the raw pipe rows of a header-only table, and
/// the spillover prose after them.
struct RenderedTableLines {
    table_lines: Vec<HyperlinkLine>,
    table_lines_prewrapped: bool,
    spillover_lines: Vec<HyperlinkLine>,
}

impl Writer<'_> {
    pub(super) fn start_table(&mut self, alignments: Vec<Alignment>) {
        self.flush_current_line();
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.table_state = Some(TableState {
            alignments,
            header: None,
            rows: Vec::new(),
            current_row: None,
            current_row_has_table_pipe_syntax: false,
            current_row_end: 0,
            current_cell: None,
            in_header: false,
        });
    }

    pub(super) fn end_table(&mut self) {
        let Some(table_state) = self.table_state.take() else {
            return;
        };
        let RenderedTableLines {
            table_lines,
            table_lines_prewrapped,
            spillover_lines,
        } = self.render_table_lines(table_state);
        let mut pending_marker_line = self.pending_marker_line;
        for line in table_lines {
            if table_lines_prewrapped {
                self.push_prewrapped_line(line, pending_marker_line);
            } else {
                self.push_line(line);
                self.flush_current_line();
            }
            pending_marker_line = false;
        }
        self.pending_marker_line = false;
        for line in spillover_lines {
            self.push_line(line);
            self.flush_current_line();
        }
        self.needs_newline = true;
    }

    pub(super) fn start_table_head(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.in_header = true;
            table_state.current_row = Some(Vec::new());
        }
    }

    pub(super) fn end_table_head(&mut self) {
        let Some(table_state) = self.table_state.as_mut() else {
            return;
        };
        if let Some(cell) = table_state.current_cell.take() {
            table_state
                .current_row
                .get_or_insert_with(Vec::new)
                .push(cell);
        }
        if let Some(row) = table_state.current_row.take() {
            table_state.header = Some(row);
        }
        table_state.in_header = false;
    }

    pub(super) fn start_table_row(&mut self, source_range: Range<usize>) {
        let source = self.input.get(source_range.clone()).unwrap_or_default();
        let has_table_pipe_syntax = {
            let source = source.trim();
            source.starts_with('|') || source.ends_with('|')
        };
        let row_end = source_range.start + source.trim_end_matches(['\n', '\r']).len();
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.current_row = Some(Vec::new());
            table_state.current_row_has_table_pipe_syntax = has_table_pipe_syntax;
            table_state.current_row_end = row_end;
        }
    }

    pub(super) fn end_table_row(&mut self) {
        let Some(table_state) = self.table_state.as_mut() else {
            return;
        };
        if let Some(cell) = table_state.current_cell.take() {
            table_state
                .current_row
                .get_or_insert_with(Vec::new)
                .push(cell);
        }
        let Some(row) = table_state.current_row.take() else {
            return;
        };
        if table_state.in_header {
            table_state.header = Some(row);
        } else {
            table_state.rows.push(TableBodyRow {
                cells: row,
                has_table_pipe_syntax: table_state.current_row_has_table_pipe_syntax,
            });
        }
        table_state.current_row_has_table_pipe_syntax = false;
    }

    /// pulldown-cmark 0.13 gives a short body row the cells it lacks, empty
    /// and past the row's text; they are left out, so that the row is the
    /// one its source has (a line without pipes is one cell, spillover).
    pub(super) fn start_table_cell(&mut self, source_range: Range<usize>) {
        if let Some(table_state) = self.table_state.as_mut() {
            let padding = !table_state.in_header
                && source_range.is_empty()
                && source_range.start >= table_state.current_row_end;
            if !padding {
                table_state.current_cell = Some(TableCell::default());
            }
        }
    }

    pub(super) fn end_table_cell(&mut self) {
        let Some(table_state) = self.table_state.as_mut() else {
            return;
        };
        if let Some(cell) = table_state.current_cell.take() {
            table_state
                .current_row
                .get_or_insert_with(Vec::new)
                .push(cell);
        }
    }

    pub(super) fn in_table_cell(&self) -> bool {
        self.table_state
            .as_ref()
            .is_some_and(|table_state| table_state.current_cell.is_some())
    }

    /// `span` in the cell being read, linked when it is a web link's.
    pub(super) fn push_span_to_table_cell(&mut self, span: Span<'static>) {
        let span = self.style_link_label(span);
        let mut part = HyperlinkLine::default();
        part.push_span(span, self.web_link_destination().as_deref());
        self.push_to_table_cell(part);
    }

    pub(super) fn push_to_table_cell(&mut self, part: HyperlinkLine) {
        if let Some(cell) = self
            .table_state
            .as_mut()
            .and_then(|table_state| table_state.current_cell.as_mut())
        {
            cell.push(part);
        }
    }

    pub(super) fn push_table_cell_hard_break(&mut self) {
        if let Some(cell) = self
            .table_state
            .as_mut()
            .and_then(|table_state| table_state.current_cell.as_mut())
        {
            cell.hard_break();
        }
    }

    pub(super) fn push_text_to_table_cell(&mut self, text: &str, prose: bool) {
        let style = self.current_inline_style();
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                self.push_table_cell_hard_break();
            }
            let span = Span::styled(line.to_string(), style);
            if self.link.is_some() || !prose {
                self.push_span_to_table_cell(span);
            } else {
                let annotated = self.style_bare_web_urls(span);
                self.push_to_table_cell(annotated);
            }
        }
    }

    /// The table's rows: the grid while its values read, else records;
    /// the raw pipe rows of a table with a header only and no room for it.
    fn render_table_lines(&self, table_state: TableState) -> RenderedTableLines {
        let column_count = table_state.alignments.len();
        let mut spillover_lines = Vec::new();
        let mut rows: Vec<Vec<TableCell>> = Vec::with_capacity(table_state.rows.len());
        for (index, row) in table_state.rows.iter().enumerate() {
            let next_row = table_state.rows.get(index + 1);
            if column_count > 1 && spillover::is_spillover_row(row, next_row) {
                if let Some(cell) = row.cells.first() {
                    spillover_lines.extend(cell.lines.iter().cloned());
                }
            } else {
                rows.push(row.cells.clone());
            }
        }
        let mut header = table_state
            .header
            .unwrap_or_else(|| vec![TableCell::default(); column_count]);
        for row in std::iter::once(&mut header).chain(rows.iter_mut()) {
            row.truncate(column_count);
            row.resize(column_count, TableCell::default());
            row.iter_mut().for_each(TableCell::ensure_line);
        }

        let metrics = collect_table_column_metrics(&header, &rows, column_count);
        let widths = compute_column_widths(&metrics, self.available_table_width(column_count));
        let header_style = crate::render::highlight::foreground_for_scopes(&[
            "entity.name.type",
            "support.type",
            "variable",
        ])
        .map_or(self.styles.strong, |color| Style::new().fg(color))
        .bold();
        let separator_style = crate::style::table_separator_style();
        let grid_widths =
            widths.filter(|widths| !key_value::should_render_records(&rows, widths, &metrics));

        let table_lines = match grid_widths {
            None if !rows.is_empty() => key_value::render_records(
                &header,
                &rows,
                &metrics,
                self.available_record_width(),
                header_style,
                separator_style,
            ),
            None => {
                return RenderedTableLines {
                    table_lines: render_table_pipe_fallback(&header, &table_state.alignments),
                    table_lines_prewrapped: false,
                    spillover_lines,
                }
            }
            Some(column_widths) => {
                let alignments = &table_state.alignments;
                let mut out = Vec::with_capacity(2 + rows.len() * 2);
                out.extend(render_table_row(
                    &header,
                    &column_widths,
                    alignments,
                    header_style,
                ));
                out.push(render_table_separator(
                    &column_widths,
                    TABLE_HEADER_SEPARATOR_CHAR,
                    separator_style,
                ));
                for (index, row) in rows.iter().enumerate() {
                    out.extend(render_table_row(
                        row,
                        &column_widths,
                        alignments,
                        Style::default(),
                    ));
                    if index + 1 < rows.len() {
                        out.push(render_table_separator(
                            &column_widths,
                            TABLE_BODY_SEPARATOR_CHAR,
                            separator_style,
                        ));
                    }
                }
                out
            }
        };
        RenderedTableLines {
            table_lines,
            table_lines_prewrapped: true,
            spillover_lines,
        }
    }

    /// The width left for the cells' content, gaps, padding and the quote
    /// and list prefixes taken off.
    fn available_table_width(&self, column_count: usize) -> Option<usize> {
        self.wrap_width.map(|wrap_width| {
            let reserved = spans_display_width(&self.prefix_spans(self.pending_marker_line))
                + column_count.saturating_sub(1) * TABLE_COLUMN_GAP
                + column_count * TABLE_CELL_PADDING * 2;
            wrap_width.saturating_sub(reserved)
        })
    }

    /// The width the records have, the prefixes taken off.
    fn available_record_width(&self) -> Option<usize> {
        self.wrap_width.map(|wrap_width| {
            wrap_width.saturating_sub(spans_display_width(
                &self.prefix_spans(self.pending_marker_line),
            ))
        })
    }
}

fn render_table_separator(
    column_widths: &[usize],
    separator_char: char,
    style: Style,
) -> HyperlinkLine {
    let segment = separator_char.to_string();
    let text = column_widths
        .iter()
        .map(|width| segment.repeat(width + TABLE_CELL_PADDING * 2))
        .collect::<Vec<_>>()
        .join(&" ".repeat(TABLE_COLUMN_GAP));
    Line::from(Span::styled(text, style)).into()
}

/// One table row: each cell wrapped to its column and aligned in it, as
/// many lines as its tallest cell. A line ends at its last cell with
/// content, so no trailing padding is written.
fn render_table_row(
    row: &[TableCell],
    column_widths: &[usize],
    alignments: &[Alignment],
    row_style: Style,
) -> Vec<HyperlinkLine> {
    let wrapped_cells: Vec<Vec<HyperlinkLine>> = row
        .iter()
        .zip(column_widths)
        .map(|(cell, width)| wrap_cell(cell, *width))
        .collect();
    let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1);

    let mut out = Vec::with_capacity(row_height);
    for row_line in 0..row_height {
        let Some(last_visible_column) = wrapped_cells.iter().rposition(|lines| {
            lines
                .get(row_line)
                .is_some_and(|line| line_display_width(&line.line) > 0)
        }) else {
            let line = wrapped_cells
                .iter()
                .find_map(|cell| cell.get(row_line))
                .cloned()
                .unwrap_or_default();
            out.push(line.style(row_style));
            continue;
        };
        let mut out_line = HyperlinkLine::default();
        for (column, width) in column_widths
            .iter()
            .enumerate()
            .take(last_visible_column + 1)
        {
            push_blank(&mut out_line, TABLE_CELL_PADDING);
            let line = wrapped_cells[column]
                .get(row_line)
                .cloned()
                .unwrap_or_default();
            let remaining = width.saturating_sub(line_display_width(&line.line));
            let (left_padding, right_padding) = match alignments[column] {
                Alignment::Left | Alignment::None => (0, remaining),
                Alignment::Center => (remaining / 2, remaining - remaining / 2),
                Alignment::Right => (remaining, 0),
            };
            if left_padding > 0 {
                push_blank(&mut out_line, left_padding);
            }
            out_line.append(line);
            if column != last_visible_column {
                push_blank(
                    &mut out_line,
                    right_padding + TABLE_CELL_PADDING + TABLE_COLUMN_GAP,
                );
            }
        }
        out.push(out_line.style(row_style));
    }
    out
}

fn push_blank(line: &mut HyperlinkLine, columns: usize) {
    line.line.push_span(Span::raw(" ".repeat(columns)));
}

/// A header-only table too wide for its columns, as its pipe source
/// (`| A | B |` and its delimiter), a cell's `|` escaped, to be wrapped
/// as prose.
fn render_table_pipe_fallback(
    header: &[TableCell],
    alignments: &[Alignment],
) -> Vec<HyperlinkLine> {
    let mut row = String::from("|");
    for cell in header {
        row.push(' ');
        row.push_str(&cell.plain_text().replace('|', "\\|"));
        row.push_str(" |");
    }
    let mut delimiter = String::from("|");
    for alignment in alignments {
        delimiter.push_str(match alignment {
            Alignment::Left => ":---",
            Alignment::Center => ":---:",
            Alignment::Right => "---:",
            Alignment::None => "---",
        });
        delimiter.push('|');
    }
    vec![Line::from(row).into(), Line::from(delimiter).into()]
}

/// A cell's content wrapped to `width`, each hard-broken line on its own,
/// its inline styles and links kept (a link split over rows is linked on
/// each); an empty cell is one empty line.
fn wrap_cell(cell: &TableCell, width: usize) -> Vec<HyperlinkLine> {
    let mut wrapped = Vec::new();
    for source_line in &cell.lines {
        let rendered = word_wrap_line_with_source(&source_line.line, RtOptions::new(width.max(1)));
        wrapped.extend(remap_source_wrapped_line(source_line, rendered));
    }
    if wrapped.is_empty() {
        wrapped.push(HyperlinkLine::default());
    }
    wrapped
}

#[cfg(test)]
mod tests;
