// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::num::NonZeroU16;

use super::*;
use crate::terminal_hyperlinks::{annotate_web_urls_in_line, mark_hyperlink_cells, HyperlinkLine};
use crate::test_support::{osc8_links, VT100Backend};
use crate::transcript_view::TextLayout;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;

fn terminal(width: u16, height: u16) -> Terminal<VT100Backend> {
    let mut terminal = Terminal::with_options(VT100Backend::new(width, height)).expect("terminal");
    terminal.set_viewport_area(Rect::new(0, 0, width, height));
    terminal
}

/// Draws `lines` laid out as the transcript lays them out, and returns
/// the bytes written.
fn draw_layout(terminal: &mut Terminal<VT100Backend>, lines: Vec<HyperlinkLine>) -> String {
    let area = terminal.viewport_area;
    let layout = TextLayout::linked(lines, area.width);
    terminal
        .draw(|frame| layout.render(area, frame.buffer_mut(), 0))
        .expect("draw");
    terminal.backend_mut().take_written()
}

#[test]
fn diff_buffers_does_not_emit_clear_to_end_for_full_width_row() {
    let area = Rect::new(0, 0, 3, 2);
    let previous = Buffer::empty(area);
    let mut next = Buffer::empty(area);

    next.cell_mut((2, 0))
        .expect("cell should exist")
        .set_symbol("X");

    let commands = diff_buffers(&previous, &next);

    let clear_count = commands
        .iter()
        .filter(|command| matches!(command, DrawCommand::ClearToEnd { y, .. } if *y == 0))
        .count();
    assert_eq!(
        0, clear_count,
        "expected diff_buffers not to emit ClearToEnd; commands: {commands:?}",
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Put { x: 2, y: 0, .. })),
        "expected diff_buffers to update the final cell; commands: {commands:?}",
    );
}

#[test]
fn diff_buffers_clear_to_end_starts_after_wide_char() {
    let area = Rect::new(0, 0, 10, 1);
    let mut previous = Buffer::empty(area);
    let mut next = Buffer::empty(area);

    previous.set_string(0, 0, "中文", Style::default());
    next.set_string(0, 0, "中", Style::default());

    let commands = diff_buffers(&previous, &next);
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::ClearToEnd { x: 2, y: 0, .. })),
        "expected clear-to-end to start after the remaining wide char; commands: {commands:?}"
    );
}

#[test]
fn a_wrapped_link_is_opened_and_closed_on_each_row() {
    let url = format!(
        "https://auth.example.com/authorize?state={}",
        "x".repeat(60)
    );
    let mut terminal = terminal(24, 8);
    let line = annotate_web_urls_in_line(Line::from(vec![
        "  ".into(),
        url.clone().cyan().underlined(),
    ]));
    let written = draw_layout(
        &mut terminal,
        vec![
            line,
            Line::from("").into(),
            Line::from("  Press Esc").into(),
        ],
    );
    let links = osc8_links(&written);
    assert!(links.len() > 1, "the URL spans rows: {links:?}");
    assert!(links.iter().all(|(destination, _)| *destination == url));
    let joined: String = links.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(joined, url);
    let footer = written.find("Press").expect("footer");
    assert!(written.rfind("\x1b]8;;\x07").expect("closed") < footer);
    let screen = terminal.backend().vt100().screen().contents();
    assert!(!screen.contains("8;;"), "{screen}");
    assert_eq!(screen.replace(['\n', ' '], "").replace("PressEsc", ""), url);
}

#[test]
fn a_link_changed_in_place_is_drawn_again() {
    let mut terminal = terminal(20, 2);
    let mut first = HyperlinkLine::default();
    first.push_span("docs".into(), Some("https://example.com/a"));
    let mut second = HyperlinkLine::default();
    second.push_span("docs".into(), Some("https://example.com/b"));
    draw_layout(&mut terminal, vec![first.clone()]);
    assert!(osc8_links(&draw_layout(&mut terminal, vec![first])).is_empty());
    assert_eq!(
        osc8_links(&draw_layout(&mut terminal, vec![second])),
        [("https://example.com/b".to_string(), "docs".to_string())]
    );
    // Unlinked again, the cells are drawn plain.
    let plain = draw_layout(&mut terminal, vec![Line::from("docs").into()]);
    assert!(osc8_links(&plain).is_empty());
    assert!(plain.contains("docs"), "{plain:?}");
}

#[test]
fn linked_cells_are_measured_by_their_visible_symbol() {
    let area = Rect::new(0, 0, 10, 1);
    let previous = Buffer::empty(area);
    let mut next = Buffer::empty(area);
    next.set_string(0, 0, "漢字 x", Style::default());
    mark_hyperlink_cells(&mut next, Rect::new(0, 0, 4, 1), "https://example.com");
    let commands = diff_buffers(&previous, &next);
    let puts: Vec<u16> = commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::Put { x, .. } => Some(*x),
            DrawCommand::ClearToEnd { .. } => None,
        })
        .collect();
    assert_eq!(puts, [0, 2, 5], "the blank between is unchanged");
    assert!(commands
        .iter()
        .any(|command| matches!(command, DrawCommand::ClearToEnd { x: 6, .. })));
}

#[test]
fn diff_buffers_clears_styled_trailing_cell_replaced_by_forced_width_cell() {
    let area = Rect::new(0, 0, 7, 1);
    let mut previous = Buffer::empty(area);
    let mut next = Buffer::empty(area);
    previous.set_string(
        0,
        0,
        "漢 tail",
        Style::default()
            .bg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
    );
    next.set_string(0, 0, "a tail", Style::default());
    next[(0, 0)]
        .set_symbol("\x1b]8;;https://example.com\x07a\x1b]8;;\x07")
        .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::MIN));

    let commands = diff_buffers(&previous, &next);

    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Put { x: 1, y: 0, .. })),
        "expected the styled trailing cell to be cleared; commands: {commands:?}"
    );
}

#[test]
fn links_take_no_columns_on_the_screen() {
    let text = "See 界ｶﾞ https://example.com/a/b/c and [x] https://example.com/d";
    let line = annotate_web_urls_in_line(Line::from(text));
    let mut terminal = terminal(16, 8);
    let written = draw_layout(&mut terminal, vec![line.clone()]);
    assert_eq!(osc8_links(&written).len(), 4, "{written:?}");
    // The screen is the buffer's visible symbols, column for column.
    let mut buffer = Buffer::empty(Rect::new(0, 0, 16, 8));
    TextLayout::linked(vec![line], 16).render(buffer.area, &mut buffer, 0);
    let mut rows = Vec::new();
    for y in 0..8 {
        let mut row = String::new();
        let mut x = 0;
        while x < 16 {
            let symbol = buffer[(x, y)].symbol();
            let visible = crate::terminal_hyperlinks::osc8_parts(symbol).map_or(symbol, |p| p.1);
            row.push_str(visible);
            x += visible.width().max(1) as u16;
        }
        rows.push(row.trim_end().to_string());
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    let screen = terminal.backend().vt100().screen().contents();
    assert_eq!(screen.lines().collect::<Vec<_>>(), rows);
}

#[test]
fn a_link_over_wide_and_halfwidth_glyphs_is_closed_at_each_cursor_move() {
    let destination = "https://example.com/rendered";
    let mut line = HyperlinkLine::new(Line::from("prefix "));
    line.push_span("漢字 ｶﾞ".into(), Some(destination));
    line.push_span(" tail".into(), None);
    let mut terminal = terminal(14, 3);
    let written = draw_layout(&mut terminal, vec![line]);
    let links = osc8_links(&written);
    assert!(links.iter().all(|(seen, _)| seen == destination));
    let joined: String = links.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(joined, "漢字 ｶﾞ");
    insta::assert_snapshot!(
        "a_link_over_wide_and_halfwidth_glyphs",
        terminal.backend().vt100().screen().contents()
    );
}
