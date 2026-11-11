// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::PendingInput;
use crate::render::renderable::Renderable;

fn render(pending: &PendingInput, width: u16) -> String {
    let area = Rect::new(0, 0, width, pending.desired_height(width));
    let mut buf = Buffer::empty(area);
    pending.render(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|text| text.to_string()).collect()
}

#[test]
fn nothing_waiting_takes_no_rows() {
    assert_eq!(PendingInput::default().desired_height(40), 0);
}

#[test]
fn each_group_has_its_header_steers_first_and_the_edit_key_last() {
    let pending = PendingInput {
        steers: strings(&["Please continue.", "Check the last command output."]),
        held: strings(&["Sent while the review ran."]),
        queued: strings(&["Queued follow-up question"]),
    };
    assert_snapshot!(render(&pending, 52));
}

#[test]
fn a_long_message_shows_three_lines_and_an_ellipsis() {
    let pending = PendingInput {
        queued: strings(&["First\nSecond\nThird\nFourth"]),
        ..PendingInput::default()
    };
    assert_snapshot!(render(&pending, 40));
}

#[test]
fn a_wrapped_steer_keeps_its_indent() {
    let pending = PendingInput {
        steers: strings(&[
            "Look at the failing test in the parser module before touching the lexer.",
        ]),
        ..PendingInput::default()
    };
    assert_snapshot!(render(&pending, 40));
}
