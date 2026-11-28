//! The picture a closing pane leaves behind: taken from where it stood, and
//! drawn narrowed as it slides out.

use super::*;
use ratatui::text::Line;
use ratatui::widgets::Widget;

fn drawn(text: &str) -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 3));
    for (row, line) in text.lines().enumerate() {
        ratatui::widgets::Paragraph::new(Line::from(line))
            .render(Rect::new(0, row as u16, 12, 1), &mut buffer);
    }
    buffer
}

#[test]
fn a_snapshot_keeps_the_pane_as_it_stood() {
    let buffer = drawn("side pane\nsecond row\nthird row");
    let snapshot = PaneSnapshot::take(Rect::new(0, 0, 12, 3), &buffer);
    assert_eq!(snapshot.area(), Rect::new(0, 0, 12, 3));
}

#[test]
fn a_narrowed_pane_shows_what_fits_of_it() {
    let buffer = drawn("side pane\nsecond row\nthird row");
    let snapshot = PaneSnapshot::take(Rect::new(0, 0, 12, 3), &buffer);
    let mut target = Buffer::empty(Rect::new(0, 0, 10, 3));
    snapshot.paint(Rect::new(0, 0, 5, 3), &mut target);
    let text = |buffer: &Buffer, y: u16| {
        (0..5)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect::<String>()
    };
    assert_eq!(text(&target, 0), "side ");
    assert_eq!(text(&target, 1), "secon");
}

#[test]
fn a_pane_that_never_drew_paints_nothing() {
    let snapshot = PaneSnapshot::take(Rect::new(0, 0, 0, 0), &Buffer::empty(Rect::new(0, 0, 4, 2)));
    let mut target = Buffer::empty(Rect::new(0, 0, 4, 2));
    snapshot.paint(Rect::new(0, 0, 4, 2), &mut target);
    assert_eq!(target, Buffer::empty(Rect::new(0, 0, 4, 2)));
}

#[test]
fn the_pane_is_drawn_where_it_now_is() {
    let buffer = drawn("side pane\nsecond row\nthird row");
    let snapshot = PaneSnapshot::take(Rect::new(0, 0, 12, 3), &buffer);
    let mut target = Buffer::empty(Rect::new(0, 0, 12, 3));
    snapshot.paint(Rect::new(6, 0, 6, 3), &mut target);
    assert_eq!(target[(6, 0)].symbol(), "s");
    assert_eq!(target[(0, 0)].symbol(), " ");
}
