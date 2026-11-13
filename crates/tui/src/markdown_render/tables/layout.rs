// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Column widths. Each column is classified from its body cells:
//! narrative (prose), token-heavy (paths, URLs, hashes) or compact (counts,
//! status words). Each starts at its widest cell; when the table is too
//! wide, token-heavy columns give up width first, then narrative ones, then
//! compact ones, each kept above a readable floor while it can be.

use ratatui::text::Line;

use super::super::output::{display_width, spans_display_width};
use super::TableCell;

/// The narrowest a column ever gets.
const MIN_COLUMN_WIDTH: usize = 3;
/// The floor kept for prose and long tokens while the table can afford it.
const READABLE_COLUMN_FLOOR: usize = 16;
/// A token this wide or wider is long: a path, a URL, a hash.
const LONG_TOKEN_WIDTH: usize = 20;

/// How a column's width is given up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableColumnKind {
    /// Prose: four words or more a cell on average, or 28 columns or more.
    Narrative,
    /// Mostly long tokens.
    TokenHeavy,
    /// Short values that should not wrap.
    Compact,
}

#[derive(Clone, Debug)]
pub(super) struct TableColumnMetrics {
    /// The widest cell, header included.
    pub(super) max_width: usize,
    /// The header's longest word.
    pub(super) header_token_width: usize,
    /// The body's longest word.
    pub(super) body_token_width: usize,
    pub(super) kind: TableColumnKind,
}

pub(super) fn collect_table_column_metrics(
    header: &[TableCell],
    rows: &[Vec<TableCell>],
    column_count: usize,
) -> Vec<TableColumnMetrics> {
    (0..column_count)
        .map(|column| {
            let header_cell = &header[column];
            let header_plain = header_cell.plain_text();
            let mut max_width = cell_display_width(header_cell);
            let mut body_token_width = 0usize;
            let mut body_token_count = 0usize;
            let mut long_body_token_count = 0usize;
            let mut total_cells = 0usize;
            let mut total_cell_width = 0usize;

            for row in rows {
                let cell = &row[column];
                max_width = max_width.max(cell_display_width(cell));
                let plain = cell.plain_text();
                let mut word_count = 0usize;
                for token in plain.split_whitespace() {
                    let token_width = display_width(token);
                    body_token_width = body_token_width.max(token_width);
                    long_body_token_count += usize::from(token_width >= LONG_TOKEN_WIDTH);
                    word_count += 1;
                }
                if word_count > 0 {
                    body_token_count += word_count;
                    total_cells += 1;
                    total_cell_width += display_width(&plain);
                }
            }

            let (avg_words_per_cell, avg_cell_width) = if total_cells == 0 {
                (
                    header_plain.split_whitespace().count() as f64,
                    display_width(&header_plain) as f64,
                )
            } else {
                (
                    body_token_count as f64 / total_cells as f64,
                    total_cell_width as f64 / total_cells as f64,
                )
            };
            let kind = if long_body_token_count > 0
                && long_body_token_count >= body_token_count.saturating_sub(long_body_token_count)
            {
                TableColumnKind::TokenHeavy
            } else if avg_words_per_cell >= 4.0 || avg_cell_width >= 28.0 {
                TableColumnKind::Narrative
            } else {
                TableColumnKind::Compact
            };

            TableColumnMetrics {
                max_width,
                header_token_width: longest_token_width(&header_plain),
                body_token_width,
                kind,
            }
        })
        .collect()
}

/// The columns' widths within `available_width`; `None` when not even
/// [`MIN_COLUMN_WIDTH`] a column fits, or the floors cannot be kept.
pub(super) fn compute_column_widths(
    metrics: &[TableColumnMetrics],
    available_width: Option<usize>,
) -> Option<Vec<usize>> {
    let mut widths: Vec<usize> = metrics
        .iter()
        .map(|column| column.max_width.max(MIN_COLUMN_WIDTH))
        .collect();
    let Some(max_width) = available_width else {
        return Some(widths);
    };
    if max_width < metrics.len() * MIN_COLUMN_WIDTH {
        return None;
    }

    let mut floors: Vec<usize> = metrics.iter().map(preferred_column_floor).collect();
    let floor_total: usize = floors.iter().sum();
    if floor_total > max_width {
        let minimums = vec![MIN_COLUMN_WIDTH; floors.len()];
        shrink_columns(&mut floors, &minimums, metrics, floor_total - max_width);
    }

    let total_width: usize = widths.iter().sum();
    if total_width > max_width
        && shrink_columns(&mut widths, &floors, metrics, total_width - max_width) > 0
    {
        return None;
    }
    Some(widths)
}

/// The width a column keeps before the others have given up theirs: the
/// readable floor for prose and long tokens; for compact values their
/// longest word (a body word counting up to the readable floor).
pub(super) fn preferred_column_floor(metrics: &TableColumnMetrics) -> usize {
    let token_target = match metrics.kind {
        TableColumnKind::Narrative | TableColumnKind::TokenHeavy => READABLE_COLUMN_FLOOR,
        TableColumnKind::Compact => metrics
            .header_token_width
            .max(metrics.body_token_width.min(READABLE_COLUMN_FLOOR)),
    };
    token_target.max(MIN_COLUMN_WIDTH).min(metrics.max_width)
}

/// Takes `amount` columns off `widths`, none below its floor: token-heavy
/// columns first, then narrative, then compact; within a kind, the one
/// with the most to spare first, so similar columns stay even. Returns what
/// could not be taken.
pub(super) fn shrink_columns(
    widths: &mut [usize],
    floors: &[usize],
    metrics: &[TableColumnMetrics],
    mut amount: usize,
) -> usize {
    for kind in [
        TableColumnKind::TokenHeavy,
        TableColumnKind::Narrative,
        TableColumnKind::Compact,
    ] {
        let slack = |idx: usize, width: usize| width.saturating_sub(floors[idx]);
        let of_kind = |idx: &usize| metrics[*idx].kind == kind;
        let slack_total: usize = (0..widths.len())
            .filter(of_kind)
            .map(|idx| slack(idx, widths[idx]))
            .sum();
        let to_remove = amount.min(slack_total);
        if to_remove == 0 {
            continue;
        }

        // The highest cap on each column's slack that removes no more than
        // `to_remove`, found by bisection; the remainder comes off the
        // columns at the cap, leftmost first.
        let mut low = 0usize;
        let mut high = (0..widths.len())
            .filter(of_kind)
            .map(|idx| slack(idx, widths[idx]))
            .max()
            .unwrap_or(0);
        while low < high {
            let cap = low + (high - low) / 2;
            let removed: usize = (0..widths.len())
                .filter(of_kind)
                .map(|idx| slack(idx, widths[idx]).saturating_sub(cap))
                .sum();
            if removed > to_remove {
                low = cap + 1;
            } else {
                high = cap;
            }
        }

        let cap = low;
        let mut removed = 0usize;
        for idx in (0..widths.len()).filter(of_kind) {
            let reduction = slack(idx, widths[idx]).saturating_sub(cap);
            widths[idx] -= reduction;
            removed += reduction;
        }
        let mut remainder = to_remove - removed;
        for idx in (0..widths.len()).filter(of_kind) {
            if remainder == 0 {
                break;
            }
            if slack(idx, widths[idx]) == cap {
                widths[idx] -= 1;
                remainder -= 1;
            }
        }

        amount -= to_remove;
        if amount == 0 {
            break;
        }
    }
    amount
}

pub(super) fn line_display_width(line: &Line<'_>) -> usize {
    spans_display_width(&line.spans)
}

pub(super) fn cell_display_width(cell: &TableCell) -> usize {
    cell.lines.iter().map(line_display_width).max().unwrap_or(0)
}

pub(super) fn longest_token_width(text: &str) -> usize {
    text.split_whitespace()
        .map(display_width)
        .max()
        .unwrap_or(0)
}
