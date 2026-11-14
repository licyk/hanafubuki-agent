// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Painting, hit testing and selection over a cell's laid-out text.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Stylize};
use ratatui::text::Line;

use super::TextLayout;
use crate::terminal_hyperlinks::{annotate_web_urls_in_line, osc8_parts, HyperlinkLine};
use crate::text_selection::SelectionUnit;

/// The cells linked on each painted row: their destination and text.
fn linked_cells(buffer: &Buffer) -> Vec<(u16, String, String)> {
    let mut out: Vec<(u16, String, String)> = Vec::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let Some((destination, visible)) = osc8_parts(buffer[(x, y)].symbol()) else {
                continue;
            };
            match out.last_mut() {
                Some((row, last, text)) if *row == y && last == destination => {
                    text.push_str(visible)
                }
                _ => out.push((y, destination.to_string(), visible.to_string())),
            }
        }
    }
    out
}

fn painted(layout: &TextLayout, area: Rect, start_row: usize) -> Buffer {
    let mut buffer = Buffer::empty(area);
    layout.render(area, &mut buffer, start_row);
    buffer
}

#[test]
fn hit_testing_uses_graphemes_and_does_not_select_padding() {
    let layout = TextLayout::new(vec!["a界e\u{301}👩‍💻".into()], 12);
    assert_eq!(
        (0..12)
            .map(|column| layout.position_at(0, column))
            .collect::<Vec<_>>(),
        vec![0, 1, 1, 4, 7, 7, 18, 18, 18, 18, 18, 18],
    );
    assert_eq!(
        &layout.text()[SelectionUnit::Word.range(layout.text(), 4)],
        "e\u{301}"
    );
}

#[test]
fn a_wide_line_wraps_at_words_then_graphemes_with_its_styles() {
    let layout = TextLayout::new(
        vec![Line::from(vec!["read ".green(), "documentation".cyan()])],
        8,
    );
    assert_eq!(layout.row_count(), 3);
    let buffer = painted(&layout, Rect::new(0, 0, 8, 2), 1);
    assert_eq!(buffer[(0, 0)].symbol(), "d");
    assert_eq!(buffer[(0, 0)].fg, Color::Cyan);
    assert_eq!(layout.position_at(1, 0), 5);
    // Copying across the wrap keeps the line whole.
    assert_eq!(layout.text(), "read documentation");
}

#[test]
fn tabs_are_expanded_to_the_columns_shown() {
    let layout = TextLayout::new(vec!["a\tb".into(), "\tc".into()], 20);
    assert_eq!(layout.text(), "a   b\n    c");
    let buffer = painted(&layout, Rect::new(0, 0, 20, 2), 0);
    assert_eq!(buffer[(4, 0)].symbol(), "b");
    assert_eq!(buffer[(4, 1)].symbol(), "c");
}

#[test]
fn selection_highlights_wide_graphemes_without_trailing_padding() {
    let layout = TextLayout::new(vec!["a界b".into()], 8);
    let area = Rect::new(0, 0, 8, 1);
    let mut buffer = painted(&layout, area, 0);
    layout.highlight(1..4, area, &mut buffer, 0);
    assert_eq!(
        (0..8)
            .map(|column| buffer[(column, 0)].modifier.contains(Modifier::REVERSED))
            .collect::<Vec<_>>(),
        vec![false, true, true, false, false, false, false, false],
    );
}

#[test]
fn selected_line_breaks_are_marked_on_nonempty_rows_only() {
    let layout = TextLayout::new(
        vec!["".into(), "alpha beta".into(), "".into(), "界".into()],
        12,
    )
    .with_disclosure_control_at("details".into(), 0)
    .with_leading_separator();
    let area = Rect::new(2, 1, 12, 9);
    let mut buffer = painted(&layout, area, 0);
    // The empty line's break, and the break after "alpha beta".
    for range in [0..1, 11..12] {
        layout.highlight_selection(range, false, area, &mut buffer, 0);
    }
    let reversed = (area.top()..area.bottom())
        .flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
        .collect::<Vec<_>>();
    assert_eq!(reversed, vec![(12, 4)]);
}

#[test]
fn offsets_can_reach_rows_after_the_u16_limit() {
    let layout = TextLayout::new(vec!["x".repeat(65_538).into()], 1);
    let buffer = painted(&layout, Rect::new(0, 0, 1, 1), 65_537);
    assert_eq!(
        (
            layout.row_count(),
            layout.row_for_offset(65_537),
            layout.position_at(65_537, 0),
            buffer[(0, 0)].symbol(),
        ),
        (65_538, 65_537, 65_537, "x"),
    );
}

#[test]
fn aligned_text_excludes_its_leading_columns() {
    let layout = TextLayout::new(vec![Line::from("wide").right_aligned()], 8);
    assert_eq!(
        (0..8)
            .map(|column| layout.position_at(0, column))
            .collect::<Vec<_>>(),
        vec![0, 0, 0, 0, 0, 1, 2, 3],
    );
    assert_eq!(layout.column_for_offset(1), 5);
}

#[test]
fn empty_lines_keep_their_hard_breaks() {
    let layout = TextLayout::new(vec!["".into(), "text".into(), "".into()], 10);
    assert_eq!(
        (
            layout.text(),
            layout.row_count(),
            SelectionUnit::Line.range(layout.text(), 0),
            SelectionUnit::Line.range(layout.text(), 5)
        ),
        ("\ntext\n", 3, 0..1, 1..6),
    );
}

#[test]
fn narrow_wrapping_keeps_joined_emoji_whole() {
    let layout = TextLayout::new(vec!["👩‍💻x".into()], 2);
    assert_eq!((layout.row_count(), layout.position_at(1, 0)), (2, 11));
    let buffer = painted(&layout, Rect::new(0, 0, 2, 2), 0);
    assert_eq!(buffer[(0, 0)].symbol(), "👩‍💻");
}

#[test]
fn spacing_and_controls_stay_out_of_the_text_and_survive_a_rewrap() {
    let layout = TextLayout::new(vec!["first line".into(), "second".into()], 20)
        .with_disclosure_control("+ Show details".into())
        .with_leading_separator();
    assert_eq!(layout.text(), "first line\nsecond");
    assert_eq!(layout.row_count(), 4);
    assert_eq!(layout.disclosure_row(), Some(3));
    assert_eq!(layout.row_for_offset(layout.text().len()), 2);
    let narrow = layout.rewrap(6);
    assert_eq!(narrow.text(), layout.text());
    assert_eq!(narrow.row_count(), 5);
    assert_eq!(narrow.disclosure_row(), Some(4));
    assert_eq!(narrow.disclosure_columns(), 1..6);
}

#[test]
fn the_caret_stays_on_graphemes_and_empty_rows() {
    let layout = TextLayout::new(vec!["ab".into(), "".into()], 10);
    assert_eq!(layout.cursor_position_at(0, 9), 1);
    assert_eq!(layout.cursor_position_at(1, 3), 3);
    assert_eq!(layout.first_cursor_position(1), None);
    assert_eq!(layout.after_cursor_grapheme(0, 1), 2);
    assert!(layout.is_cursor_position(1));
    assert!(!layout.is_cursor_position(2));
}

#[test]
fn tabs_move_links_with_the_text_after_them() {
    let mut line = HyperlinkLine::new(Line::from("a\t"));
    line.push_span("docs".into(), Some("https://example.com/docs"));
    line.push_span("\tafter".into(), None);
    let layout = TextLayout::linked(vec![line], 20);
    assert_eq!(layout.text(), "a   docs    after");
    let buffer = painted(&layout, Rect::new(0, 0, 20, 1), 0);
    assert_eq!(
        linked_cells(&buffer),
        [(
            0,
            "https://example.com/docs".to_string(),
            "docs".to_string()
        )]
    );
    assert!(osc8_parts(buffer[(3, 0)].symbol()).is_none());
    assert!(osc8_parts(buffer[(8, 0)].symbol()).is_none());
}

#[test]
fn a_url_wider_than_the_view_is_cut_at_the_width_and_linked_on_each_row() {
    let url = "https://example.com/a-very-long-path-with-many-segments";
    let line = annotate_web_urls_in_line(Line::from(format!("• {url}")));
    for width in 8..40 {
        let layout = TextLayout::linked(vec![line.clone()], width);
        let area = Rect::new(0, 0, width, layout.row_count() as u16);
        let buffer = painted(&layout, area, 0);
        let linked = linked_cells(&buffer);
        assert_eq!(
            linked
                .iter()
                .map(|(_, _, text)| text.as_str())
                .collect::<String>(),
            url,
            "{width}"
        );
        // The bullet stays on the URL's row, which is cut at the width as
        // a terminal wraps it: every row but the last is full.
        let rows = layout.row_count();
        assert_eq!(
            rows,
            (2 + url.len()).div_ceil(usize::from(width)),
            "{width}"
        );
        for row in 0..rows - 1 {
            assert_eq!(layout.row_line(row).width(), usize::from(width), "{width}");
        }
        // The copied text is the URL whole.
        assert_eq!(layout.text(), format!("• {url}"));
    }
}

#[test]
fn a_row_line_carries_its_links_after_its_indent() {
    let mut line = HyperlinkLine::default();
    line.push_span("docs".into(), Some("https://example.com"));
    line.line = line.line.right_aligned();
    let layout = TextLayout::linked(vec![line], 10);
    let row = layout.row_line(0);
    assert_eq!(row.line.to_string(), "      docs");
    assert_eq!(row.hyperlinks[0].columns, 6..10);
}
