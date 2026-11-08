//! The inline viewport on an emulated terminal: growing it scrolls the
//! transcript up (ratatui's scrolling regions, upstream), and history is
//! written above it.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::Tui;
use crate::custom_terminal::Terminal;
use crate::test_support::VT100Backend;

fn rows(tui: &Tui<VT100Backend>) -> Vec<String> {
    let screen = tui.terminal.backend().vt100().screen();
    let (height, width) = screen.size();
    (0..height)
        .map(|row| {
            screen
                .contents_between(row, 0, row, width)
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn a_growing_viewport_scrolls_the_transcript_up_and_history_goes_above_it() {
    let mut terminal = Terminal::with_options(VT100Backend::new(20, 8)).unwrap();
    terminal.set_viewport_area(Rect::new(0, 7, 20, 1));
    let mut tui = Tui::new(terminal);
    tui.insert_history_lines(vec![Line::from("first"), Line::from("second")]);
    tui.draw(1, |frame| {
        let area = frame.area();
        Paragraph::new("composer").render(area, frame.buffer_mut());
    })
    .unwrap();
    assert_eq!(rows(&tui)[5..], ["first", "second", "composer"]);

    tui.draw(3, |frame| {
        let area = frame.area();
        Paragraph::new(vec![Line::from("a"), Line::from("b"), Line::from("c")])
            .render(area, frame.buffer_mut());
    })
    .unwrap();
    assert_eq!(rows(&tui)[3..], ["first", "second", "a", "b", "c"]);
    assert_eq!(tui.terminal.viewport_area, Rect::new(0, 5, 20, 3));
}
