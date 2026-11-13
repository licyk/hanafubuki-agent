// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use pulldown_cmark::Alignment;
use ratatui::style::Style;
use ratatui::text::Line;

use super::layout::{
    cell_display_width, collect_table_column_metrics, compute_column_widths, longest_token_width,
    preferred_column_floor, shrink_columns, TableColumnKind, TableColumnMetrics,
};
use super::spillover::is_spillover_row;
use super::{render_table_row, wrap_cell, TableBodyRow, TableCell};

fn make_cell(text: &str) -> TableCell {
    let mut cell = TableCell::default();
    cell.push_span(text.to_string().into());
    cell
}

fn make_body_row(cells: Vec<TableCell>, has_table_pipe_syntax: bool) -> TableBodyRow {
    TableBodyRow {
        cells,
        has_table_pipe_syntax,
    }
}

fn metrics(
    max_width: usize,
    header: usize,
    body: usize,
    kind: TableColumnKind,
) -> TableColumnMetrics {
    TableColumnMetrics {
        max_width,
        header_token_width: header,
        body_token_width: body,
        kind,
    }
}

#[test]
fn wrap_cell_preserves_hard_break_lines() {
    let mut cell = make_cell("first line");
    cell.hard_break();
    cell.push_span("second line".into());
    let rendered: Vec<String> = wrap_cell(&cell, 40)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(rendered, vec!["first line", "second line"]);
}

#[test]
fn blank_table_continuations_do_not_repeat_the_first_cell() {
    let mut cell = make_cell("first");
    cell.hard_break();
    cell.hard_break();
    cell.push_span("last".into());
    cell.hard_break();
    let lines = render_table_row(
        &[make_cell("short"), cell],
        &[5, 5],
        &[Alignment::None, Alignment::None],
        Style::default(),
    );
    let rendered: Vec<String> = lines.iter().map(Line::to_string).collect();
    insta::assert_debug_snapshot!(rendered);
}

#[test]
fn column_classification_narrative_by_word_count() {
    let header = vec![make_cell("ID"), make_cell("Description")];
    let rows = vec![
        vec![make_cell("1"), make_cell("a long description of the item")],
        vec![make_cell("2"), make_cell("another verbose body cell here")],
    ];
    let metrics = collect_table_column_metrics(&header, &rows, 2);
    assert_eq!(metrics[0].kind, TableColumnKind::Compact);
    assert_eq!(metrics[1].kind, TableColumnKind::Narrative);
}

#[test]
fn column_classification_token_heavy_by_url_like_tokens() {
    let header = vec![make_cell("URL")];
    let rows = vec![
        vec![make_cell("https://example.com/very/long/path")],
        vec![make_cell("https://another.example.org/deep")],
    ];
    let metrics = collect_table_column_metrics(&header, &rows, 1);
    assert_eq!(metrics[0].kind, TableColumnKind::TokenHeavy);
}

#[test]
fn column_classification_token_heavy_for_local_path_lists() {
    let header = vec![make_cell("Files")];
    let rows = vec![
        vec![make_cell(
            "codex-rs/core/src/next_prompt_suggestion.rs:1, codex-rs/core/src/next_prompt_suggestion_tests.rs:1",
        )],
        vec![make_cell(
            "codex-rs/core/src/context/next_prompt_suggestion.rs:1, codex-rs/core/src/context/contextual_user_message_tests.rs:1",
        )],
    ];
    let metrics = collect_table_column_metrics(&header, &rows, 1);
    assert_eq!(metrics[0].kind, TableColumnKind::TokenHeavy);
}

#[test]
fn column_classification_compact_all_short() {
    let header = vec![make_cell("Status"), make_cell("Count")];
    let rows = vec![
        vec![make_cell("ok"), make_cell("42")],
        vec![make_cell("err"), make_cell("7")],
    ];
    let metrics = collect_table_column_metrics(&header, &rows, 2);
    assert_eq!(metrics[0].kind, TableColumnKind::Compact);
    assert_eq!(metrics[1].kind, TableColumnKind::Compact);
}

#[test]
fn preferred_floor_narrative_retains_readable_width() {
    assert_eq!(
        preferred_column_floor(&metrics(40, 15, 8, TableColumnKind::Narrative)),
        16
    );
    assert_eq!(
        preferred_column_floor(&metrics(12, 6, 8, TableColumnKind::Narrative)),
        12
    );
}

#[test]
fn preferred_floor_token_heavy_retains_readable_width() {
    assert_eq!(
        preferred_column_floor(&metrics(80, 5, 60, TableColumnKind::TokenHeavy)),
        16
    );
}

#[test]
fn preferred_floor_compact_uses_body_token() {
    assert_eq!(
        preferred_column_floor(&metrics(30, 5, 12, TableColumnKind::Compact)),
        12
    );
    assert_eq!(
        preferred_column_floor(&metrics(30, 5, 20, TableColumnKind::Compact)),
        16
    );
}

#[test]
fn bulk_column_shrink_matches_one_cell_at_a_time() {
    fn priority(kind: TableColumnKind) -> usize {
        match kind {
            TableColumnKind::TokenHeavy => 0,
            TableColumnKind::Narrative => 1,
            TableColumnKind::Compact => 2,
        }
    }

    fn shrink_one_at_a_time(
        widths: &mut [usize],
        floors: &[usize],
        metrics: &[TableColumnMetrics],
        mut amount: usize,
    ) -> usize {
        while amount > 0 {
            let Some(idx) = widths
                .iter()
                .enumerate()
                .filter(|(idx, width)| **width > floors[*idx])
                .min_by_key(|(idx, width)| {
                    (
                        priority(metrics[*idx].kind),
                        usize::MAX - width.saturating_sub(floors[*idx]),
                    )
                })
                .map(|(idx, _)| idx)
            else {
                break;
            };
            widths[idx] -= 1;
            amount -= 1;
        }
        amount
    }

    for case in 0..64usize {
        let metrics = (0..5)
            .map(|idx| {
                let kind = match (case + idx) % 3 {
                    0 => TableColumnKind::TokenHeavy,
                    1 => TableColumnKind::Narrative,
                    _ => TableColumnKind::Compact,
                };
                metrics(100, 8, 8, kind)
            })
            .collect::<Vec<_>>();
        let floors = (0..5)
            .map(|idx| 3 + (case * (idx + 1) + idx) % 14)
            .collect::<Vec<_>>();
        let initial = floors
            .iter()
            .enumerate()
            .map(|(idx, floor)| floor + (case * (idx + 3) + idx * 7) % 41)
            .collect::<Vec<_>>();
        let slack = initial
            .iter()
            .zip(&floors)
            .map(|(width, floor)| width - floor)
            .sum::<usize>();

        for amount in 0..=slack + 1 {
            let mut expected = initial.clone();
            let expected_remaining = shrink_one_at_a_time(&mut expected, &floors, &metrics, amount);
            let mut actual = initial.clone();
            let actual_remaining = shrink_columns(&mut actual, &floors, &metrics, amount);
            assert_eq!((actual, actual_remaining), (expected, expected_remaining));
        }
    }
}

#[test]
fn column_widths_bulk_shrink_large_token_heavy_column() {
    let metrics = [
        metrics(1_000_000, 4, 1_000_000, TableColumnKind::TokenHeavy),
        metrics(8, 5, 8, TableColumnKind::Compact),
    ];
    assert_eq!(compute_column_widths(&metrics, Some(24)), Some(vec![16, 8]));
}

#[test]
fn spillover_detects_single_cell_row() {
    let row = make_body_row(vec![make_cell("some trailing text")], false);
    assert!(is_spillover_row(&row, None));
}

#[test]
fn spillover_keeps_single_cell_row_with_table_pipe_syntax() {
    let row = make_body_row(vec![make_cell("some sparse value")], true);
    assert!(!is_spillover_row(&row, None));
}

#[test]
fn spillover_detects_html_content() {
    let row = make_body_row(
        vec![
            make_cell("<div>content</div>"),
            make_cell(""),
            make_cell(""),
        ],
        false,
    );
    assert!(is_spillover_row(&row, None));
}

#[test]
fn spillover_detects_label_followed_by_html() {
    let row = make_body_row(
        vec![make_cell("HTML block:"), make_cell(""), make_cell("")],
        false,
    );
    let next = make_body_row(
        vec![make_cell("<div>x</div>"), make_cell(""), make_cell("")],
        false,
    );
    assert!(is_spillover_row(&row, Some(&next)));
}

#[test]
fn spillover_detects_trailing_html_label() {
    let row = make_body_row(
        vec![make_cell("HTML block:"), make_cell(""), make_cell("")],
        false,
    );
    assert!(is_spillover_row(&row, None));
}

#[test]
fn spillover_keeps_normal_multi_cell_row() {
    let row = make_body_row(
        vec![make_cell("one"), make_cell("two"), make_cell("three")],
        true,
    );
    assert!(!is_spillover_row(&row, None));
}

#[test]
fn spillover_keeps_label_when_next_is_not_html() {
    let row = make_body_row(
        vec![make_cell("Status:"), make_cell(""), make_cell("")],
        true,
    );
    let next = make_body_row(vec![make_cell("ok"), make_cell(""), make_cell("")], true);
    assert!(!is_spillover_row(&row, Some(&next)));
}

#[test]
fn table_widths_count_halfwidth_sound_marks() {
    assert_eq!(cell_display_width(&make_cell("ｶﾞﾊﾟ")), 4);
    assert_eq!(longest_token_width("ｶﾞﾊﾟtail"), 8);
}
