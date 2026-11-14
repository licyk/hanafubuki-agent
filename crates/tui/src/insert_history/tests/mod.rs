// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

mod vt100_history;

use super::*;
use crate::markdown_render::render_markdown_text;
use crate::terminal_hyperlinks::annotate_web_urls_in_line;
use crate::test_support::{osc8_links, VT100Backend};
use ratatui::layout::Rect;
use ratatui::style::Color;

#[test]
fn writes_bold_then_regular_spans() {
    use ratatui::style::Stylize;

    let spans = ["A".bold(), "B".into()];

    let mut actual: Vec<u8> = Vec::new();
    write_spans(&mut actual, spans.iter()).unwrap();

    let mut expected: Vec<u8> = Vec::new();
    queue!(
        expected,
        SetAttribute(ratatui::crossterm::style::Attribute::Bold),
        Print("A"),
        SetAttribute(ratatui::crossterm::style::Attribute::NormalIntensity),
        Print("B"),
        SetForegroundColor(CColor::Reset),
        SetBackgroundColor(CColor::Reset),
        SetAttribute(ratatui::crossterm::style::Attribute::Reset),
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(actual).unwrap(),
        String::from_utf8(expected).unwrap()
    );
}

#[test]
fn vt100_blockquote_line_emits_green_fg() {
    // Set up a small off-screen terminal
    let width: u16 = 40;
    let height: u16 = 10;
    let backend = VT100Backend::new(width, height);
    let mut term = crate::custom_terminal::Terminal::with_options(backend).expect("terminal");
    // Place viewport on the last line so history inserts scroll upward
    let viewport = Rect::new(0, height - 1, width, 1);
    term.set_viewport_area(viewport);

    // Build a blockquote-like line: apply line-level green style and prefix "> "
    let mut line: Line<'static> = Line::from(vec!["> ".into(), "Hello world".into()]);
    line = line.style(Color::Green);
    insert_history_lines(&mut term, vec![line]).expect("Failed to insert history lines in test");

    let mut saw_colored = false;
    'outer: for row in 0..height {
        for col in 0..width {
            if term
                .backend()
                .vt100()
                .screen()
                .cell(row, col)
                .is_some_and(|cell| cell.has_contents() && cell.fgcolor() != vt100::Color::Default)
            {
                saw_colored = true;
                break 'outer;
            }
        }
    }
    assert!(
        saw_colored,
        "expected at least one colored cell in vt100 output"
    );
}

#[test]
fn vt100_blockquote_wrap_preserves_color_on_all_wrapped_lines() {
    // Force wrapping by using a narrow viewport width and a long blockquote line.
    let width: u16 = 20;
    let height: u16 = 8;
    let backend = VT100Backend::new(width, height);
    let mut term = crate::custom_terminal::Terminal::with_options(backend).expect("terminal");
    // Viewport is the last line so history goes directly above it.
    let viewport = Rect::new(0, height - 1, width, 1);
    term.set_viewport_area(viewport);

    // Create a long blockquote with a distinct prefix and enough text to wrap.
    let mut line: Line<'static> = Line::from(vec![
        "> ".into(),
        "This is a long quoted line that should wrap".into(),
    ]);
    line = line.style(Color::Green);

    insert_history_lines(&mut term, vec![line]).expect("Failed to insert history lines in test");

    // Parse and inspect the final screen buffer.
    let screen = term.backend().vt100().screen();

    // Collect rows that are non-empty; these should correspond to our wrapped lines.
    let mut non_empty_rows: Vec<u16> = Vec::new();
    for row in 0..height {
        let mut any = false;
        for col in 0..width {
            if screen.cell(row, col).is_some_and(|cell| {
                cell.has_contents() && cell.contents() != "\0" && cell.contents() != " "
            }) {
                any = true;
                break;
            }
        }
        if any {
            non_empty_rows.push(row);
        }
    }

    // Expect at least two rows due to wrapping.
    assert!(
        non_empty_rows.len() >= 2,
        "expected wrapped output to span >=2 rows, got {non_empty_rows:?}",
    );

    // For each non-empty row, ensure all non-space cells are using a non-default fg color.
    for row in non_empty_rows {
        for col in 0..width {
            if let Some(cell) = screen.cell(row, col) {
                let contents = cell.contents();
                if !contents.is_empty() && contents != " " {
                    assert!(
                        cell.fgcolor() != vt100::Color::Default,
                        "expected non-default fg on row {row} col {col}, got {:?}",
                        cell.fgcolor()
                    );
                }
            }
        }
    }
}

#[test]
fn vt100_colored_prefix_then_plain_text_resets_color() {
    let width: u16 = 40;
    let height: u16 = 6;
    let backend = VT100Backend::new(width, height);
    let mut term = crate::custom_terminal::Terminal::with_options(backend).expect("terminal");
    let viewport = Rect::new(0, height - 1, width, 1);
    term.set_viewport_area(viewport);

    // First span colored, rest plain.
    let line: Line<'static> = Line::from(vec![
        Span::styled("1. ", ratatui::style::Style::default().fg(Color::LightBlue)),
        Span::raw("Hello world"),
    ]);

    insert_history_lines(&mut term, vec![line]).expect("Failed to insert history lines in test");

    let screen = term.backend().vt100().screen();

    // Find the first non-empty row; verify first three cells are colored, following cells default.
    'rows: for row in 0..height {
        let mut has_text = false;
        for col in 0..width {
            if screen
                .cell(row, col)
                .is_some_and(|cell| cell.has_contents() && cell.contents() != " ")
            {
                has_text = true;
                break;
            }
        }
        if !has_text {
            continue;
        }

        // Expect "1. Hello world" starting at col 0.
        for col in 0..3 {
            let cell = screen.cell(row, col).unwrap();
            assert!(
                cell.fgcolor() != vt100::Color::Default,
                "expected colored prefix at col {col}, got {:?}",
                cell.fgcolor()
            );
        }
        for col in 3..(3 + "Hello world".len() as u16) {
            let cell = screen.cell(row, col).unwrap();
            assert_eq!(
                cell.fgcolor(),
                vt100::Color::Default,
                "expected default color for plain text at col {col}, got {:?}",
                cell.fgcolor()
            );
        }
        break 'rows;
    }
}

#[test]
fn vt100_deep_nested_mixed_list_third_level_marker_is_colored() {
    // Markdown with five levels (ordered → unordered → ordered → unordered → unordered).
    let md = "1. First\n   - Second level\n     1. Third level (ordered)\n        - Fourth level (bullet)\n          - Fifth level to test indent consistency\n";
    let text = render_markdown_text(md);
    let lines: Vec<Line<'static>> = text.lines.clone();

    let width: u16 = 60;
    let height: u16 = 12;
    let backend = VT100Backend::new(width, height);
    let mut term = crate::custom_terminal::Terminal::with_options(backend).expect("terminal");
    let viewport = ratatui::layout::Rect::new(0, height - 1, width, 1);
    term.set_viewport_area(viewport);

    insert_history_lines(&mut term, lines).expect("Failed to insert history lines in test");

    let screen = term.backend().vt100().screen();

    // Reconstruct screen rows as strings to locate the 3rd level line.
    let rows: Vec<String> = screen.rows(0, width).collect();

    let needle = "1. Third level (ordered)";
    let row_idx = rows
        .iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| {
            panic!("expected to find row containing {needle:?}, have rows: {rows:?}")
        });
    let col_start = rows[row_idx].find(needle).unwrap() as u16; // column where '1' starts

    // Verify that the numeric marker ("1.") at the third level is colored
    // (non-default fg) and the content after the following space resets to default.
    for c in [col_start, col_start + 1] {
        let cell = screen.cell(row_idx as u16, c).unwrap();
        assert!(
            cell.fgcolor() != vt100::Color::Default,
            "expected colored 3rd-level marker at row {row_idx} col {c}, got {:?}",
            cell.fgcolor()
        );
    }
    let content_col = col_start + 3; // skip '1', '.', and the space
    if let Some(cell) = screen.cell(row_idx as u16, content_col) {
        assert_eq!(
            cell.fgcolor(),
            vt100::Color::Default,
            "expected default color for 3rd-level content at row {row_idx} col {content_col}, got {:?}",
            cell.fgcolor()
        );
    }
}

fn history_terminal(width: u16, height: u16) -> crate::custom_terminal::Terminal<VT100Backend> {
    let backend = VT100Backend::new(width, height);
    let mut term = crate::custom_terminal::Terminal::with_options(backend).expect("terminal");
    term.set_viewport_area(Rect::new(0, height - 1, width, 1));
    term
}

#[test]
fn a_linked_row_is_written_as_an_osc8_link_closed_on_its_row() {
    let mut term = history_terminal(40, 6);
    let mut line = HyperlinkLine::new(Line::from("> See "));
    line.push_span("the docs".into(), Some("https://example.com/docs"));
    line.push_span(" and ".into(), None);
    line.push_span("界ｶﾞ".into(), Some("https://example.com/wide"));
    insert_history_lines(&mut term, vec![line.style(Color::Green.into())]).expect("insert");
    let written = term.backend_mut().take_written();
    assert_eq!(
        osc8_links(&written),
        [
            (
                "https://example.com/docs".to_string(),
                "the docs".to_string()
            ),
            ("https://example.com/wide".to_string(), "界ｶﾞ".to_string()),
        ]
    );
    let screen = term.backend().vt100().screen().contents();
    assert!(screen.contains("> See the docs and 界ｶﾞ"), "{screen}");
}

#[test]
fn a_link_the_writer_wraps_is_linked_on_each_row() {
    let url = "https://example.com/a/rather/long/path/to/the/page";
    let mut term = history_terminal(20, 8);
    insert_history_lines(&mut term, vec![annotate_web_urls_in_line(Line::from(url))])
        .expect("insert");
    let links = osc8_links(&term.backend_mut().take_written());
    assert!(links.len() > 1, "{links:?}");
    assert_eq!(
        links
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<String>(),
        url
    );
}

#[test]
fn a_hostile_row_writes_no_control_sequence_of_its_own() {
    let mut term = history_terminal(60, 4);
    let mut line = HyperlinkLine::new(Line::from("\x1b]8;;https://evil.test\x07x\x1b[2J\u{9c}"));
    line.push_span(
        "click\x1b]0;title\x07".into(),
        Some("https://example.com/\x1b]8;;https://evil.test\x07\u{9b}"),
    );
    insert_history_lines(&mut term, vec![line]).expect("insert");
    let written = term.backend_mut().take_written();
    assert_eq!(
        osc8_links(&written),
        [(
            "https://example.com/]8;;https://evil.test".to_string(),
            "click]0;title".to_string()
        )]
    );
    // The link's own two BELs, and no other.
    assert_eq!(written.matches('\x07').count(), 2, "{written:?}");
    assert!(
        !written.contains("\x1b[2J") && !written.contains("\x1b]0;"),
        "{written:?}"
    );
}
