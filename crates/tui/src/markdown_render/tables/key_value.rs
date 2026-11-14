// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A table too narrow to read as a grid, as records: each body row a block
//! of `header  value` fields, the records apart by a rule. Labels and
//! values side by side while the width allows, else each label on its own
//! line with its value indented under it.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::super::output::display_width;
use super::layout::{cell_display_width, line_display_width, TableColumnKind, TableColumnMetrics};
use super::{wrap_cell, TableCell, TABLE_BODY_SEPARATOR_CHAR};
use crate::render::line_utils::line_to_static;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::wrapping::{word_wrap_line, RtOptions};

const FIELD_LEADING_PADDING: usize = 1;
const FIELD_GAP: usize = 2;
const MIN_VALUE_WIDTH: usize = 3;
const MIN_ALIGNED_COMPACT_VALUE_WIDTH: usize = 12;
const MIN_ALIGNED_EXPANSIVE_VALUE_WIDTH: usize = 24;
const MIN_SCANNABLE_NARRATIVE_WIDTH: usize = 12;
const MIN_SCANNABLE_TOKEN_HEAVY_WIDTH: usize = 12;
const CRAMPED_EXPANSIVE_CELL_LINES: usize = 4;
const CATASTROPHIC_NARRATIVE_CELL_LINES: usize = 7;
const STACKED_VALUE_INDENT: usize = 2;

/// Whether enough rows (one of one, else a third and at least two) have a
/// value the grid breaks apart: a compact word split, a long token split in
/// a narrow column, or prose squeezed into a tall strip.
pub(super) fn should_render_records(
    rows: &[Vec<TableCell>],
    column_widths: &[usize],
    metrics: &[TableColumnMetrics],
) -> bool {
    if rows.is_empty() {
        return false;
    }
    let affected_rows = rows
        .iter()
        .filter(|row| {
            let contains_fragmented_value =
                row.iter()
                    .zip(column_widths)
                    .zip(metrics)
                    .any(|((cell, width), metrics)| {
                        let has_fragmented_token = cell
                            .plain_text()
                            .split_whitespace()
                            .any(|token| display_width(token) > *width);
                        match metrics.kind {
                            TableColumnKind::Compact => has_fragmented_token,
                            TableColumnKind::TokenHeavy => {
                                *width < MIN_SCANNABLE_TOKEN_HEAVY_WIDTH && has_fragmented_token
                            }
                            TableColumnKind::Narrative => false,
                        }
                    });
            contains_fragmented_value || expansive_cells_are_starved(row, column_widths, metrics)
        })
        .count();
    let threshold = if rows.len() == 1 {
        1
    } else {
        2.max(rows.len().div_ceil(3))
    };
    affected_rows >= threshold
}

/// Two prose or token cells four lines tall or more, or a narrow prose
/// cell seven lines tall or more.
fn expansive_cells_are_starved(
    row: &[TableCell],
    column_widths: &[usize],
    metrics: &[TableColumnMetrics],
) -> bool {
    let expansive_cells: Vec<(TableColumnKind, usize, usize)> = row
        .iter()
        .zip(column_widths)
        .zip(metrics)
        .filter(|(_, metrics)| metrics.kind != TableColumnKind::Compact)
        .map(|((cell, width), metrics)| (metrics.kind, *width, wrap_cell(cell, *width).len()))
        .collect();
    expansive_cells
        .iter()
        .filter(|(_, _, height)| *height >= CRAMPED_EXPANSIVE_CELL_LINES)
        .count()
        >= 2
        || expansive_cells.iter().any(|(kind, width, height)| {
            *kind == TableColumnKind::Narrative
                && *width < MIN_SCANNABLE_NARRATIVE_WIDTH
                && *height >= CATASTROPHIC_NARRATIVE_CELL_LINES
        })
}

pub(super) fn render_records(
    headers: &[TableCell],
    rows: &[Vec<TableCell>],
    metrics: &[TableColumnMetrics],
    available_width: Option<usize>,
    label_style: Style,
    separator_style: Style,
) -> Vec<HyperlinkLine> {
    let label_width = headers
        .iter()
        .map(|header| display_width(&header.plain_text()))
        .max()
        .unwrap_or(0);
    let minimum_value_width = if metrics
        .iter()
        .any(|metrics| metrics.kind != TableColumnKind::Compact)
    {
        MIN_ALIGNED_EXPANSIVE_VALUE_WIDTH
    } else {
        MIN_ALIGNED_COMPACT_VALUE_WIDTH
    };
    let aligned_fields = available_width.is_none_or(|width| {
        FIELD_LEADING_PADDING + label_width + FIELD_GAP + minimum_value_width <= width
    });
    let mut out = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (header, value) in headers.iter().zip(row) {
            if aligned_fields {
                render_aligned_field(
                    &mut out,
                    header,
                    value,
                    label_width,
                    available_width,
                    label_style,
                );
            } else {
                render_stacked_field(&mut out, header, value, available_width, label_style);
            }
        }
        if row_index + 1 < rows.len() {
            let width = available_width.unwrap_or_else(|| {
                out.iter()
                    .map(|line| line_display_width(&line.line))
                    .max()
                    .unwrap_or(0)
            });
            out.push(
                Line::from(Span::styled(
                    TABLE_BODY_SEPARATOR_CHAR.to_string().repeat(width),
                    separator_style,
                ))
                .into(),
            );
        }
    }
    out
}

/// ` label   value`, the value wrapped in the column after the widest label.
fn render_aligned_field(
    out: &mut Vec<HyperlinkLine>,
    header: &TableCell,
    value: &TableCell,
    label_width: usize,
    available_width: Option<usize>,
    label_style: Style,
) {
    let value_indent = FIELD_LEADING_PADDING + label_width + FIELD_GAP;
    let value_width = available_width
        .map(|width| width.saturating_sub(value_indent).max(MIN_VALUE_WIDTH))
        .unwrap_or_else(|| cell_display_width(value).max(MIN_VALUE_WIDTH));
    for (line_index, value_line) in wrap_cell(value, value_width).into_iter().enumerate() {
        let spans = if line_index == 0 {
            let label = header.plain_text();
            let padding = label_width.saturating_sub(display_width(&label)) + FIELD_GAP;
            vec![
                Span::raw(" ".repeat(FIELD_LEADING_PADDING)),
                Span::styled(label, label_style),
                Span::raw(" ".repeat(padding)),
            ]
        } else {
            vec![Span::raw(" ".repeat(value_indent))]
        };
        out.push(value_line.prefixed(spans));
    }
}

/// ` label` on its own lines, the value under it indented.
fn render_stacked_field(
    out: &mut Vec<HyperlinkLine>,
    header: &TableCell,
    value: &TableCell,
    available_width: Option<usize>,
    label_style: Style,
) {
    let label_width = available_width
        .map(|width| width.saturating_sub(FIELD_LEADING_PADDING).max(1))
        .unwrap_or_else(|| display_width(&header.plain_text()).max(1));
    let label = Line::from(Span::styled(header.plain_text(), label_style));
    for label_line in word_wrap_line(&label, RtOptions::new(label_width)) {
        let mut spans = vec![Span::raw(" ".repeat(FIELD_LEADING_PADDING))];
        spans.extend(line_to_static(&label_line).spans);
        out.push(Line::from(spans).into());
    }

    let value_width = available_width
        .map(|width| width.saturating_sub(STACKED_VALUE_INDENT).max(1))
        .unwrap_or_else(|| cell_display_width(value).max(1));
    for value_line in wrap_cell(value, value_width) {
        out.push(value_line.prefixed(vec![Span::raw(" ".repeat(STACKED_VALUE_INDENT))]));
    }
}
