//! The tail as the view paints it: from its first row, its last rows under
//! a header when it is taller, and the controls reached by a click and F4
//! without it ever scrolling.

use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::transcript_view::tests::fixtures::{cell, key, live, text, tool_call};

fn paint(
    view: &mut TranscriptView,
    cells: &[Arc<dyn HistoryCell>],
    width: u16,
    height: u16,
    separated: bool,
) -> String {
    view.measure_tail(cells, width, separated);
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    view.render_tail(area, &mut buffer, cells);
    text(&buffer)
}

#[test]
fn the_tail_is_painted_from_its_first_row_separated_from_what_was_written() {
    let mut view = TranscriptView::default();
    let cells = [cell("first"), cell("second")];
    assert_eq!(
        paint(&mut view, &cells, 20, 5, false),
        "first\n\nsecond\n\n"
    );
    assert_eq!(paint(&mut view, &cells, 20, 5, true), "\nfirst\n\nsecond\n");
}

#[test]
fn a_tail_taller_than_its_area_shows_its_last_rows_under_their_entrys_header() {
    let mut view = TranscriptView::default();
    let running = live("Thinking\nline 1\nline 2\nline 3\nline 4\nline 5");
    let cells: [Arc<dyn HistoryCell>; 1] = [running];
    assert_eq!(
        paint(&mut view, &cells, 20, 3, false),
        "Thinking\nline 4\nline 5"
    );
}

#[test]
fn a_click_on_a_control_toggles_it_and_the_tail_stays_at_its_latest_rows() {
    let mut view = TranscriptView::default();
    let mut cells = vec![tool_call("call-1", 20)];
    let screen = paint(&mut view, &cells, 40, 30, false);
    assert!(!screen.contains("out 10"), "{screen}");
    let row = screen
        .lines()
        .position(|line| line.contains("+ 10 lines"))
        .expect(&screen);
    let column = screen.lines().nth(row).unwrap().find('+').unwrap();
    assert!(view.click_tail(
        &cells,
        u16::try_from(column).unwrap(),
        u16::try_from(row).unwrap()
    ));
    let screen = paint(&mut view, &cells, 40, 30, false);
    assert!(screen.contains("out 10"), "{screen}");
    assert!(screen.contains("− Show less"), "{screen}");

    // An entry added after the toggle shows, as the newest row.
    cells.push(cell("after the toggle"));
    let screen = paint(&mut view, &cells, 40, 8, false);
    assert!(screen.trim_end().ends_with("after the toggle"), "{screen}");
    // A click elsewhere toggles nothing.
    assert!(!view.click_tail(&cells, 0, 7));
}

#[test]
fn f4_focuses_the_tails_activity_and_enter_expands_it_in_place() {
    let mut view = TranscriptView::default();
    let cells = vec![cell("note"), tool_call("call-1", 20)];
    paint(&mut view, &cells, 40, 30, false);
    assert!(view.handle_tail_key(key(KeyCode::F(4)), &cells).is_some());
    assert!(view.handle_tail_key(key(KeyCode::Enter), &cells).is_some());
    let screen = paint(&mut view, &cells, 40, 30, false);
    assert!(screen.starts_with("note"), "{screen}");
    assert!(screen.contains("out 10"), "{screen}");
    // Typing goes back to the composer.
    assert!(view
        .handle_tail_key(key(KeyCode::Char('a')), &cells)
        .is_none());
}
