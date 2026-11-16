// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Keyboard help for the live mention popup; filter tabs live above the results.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Widget;

use crate::key_hint;
use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;

pub(super) fn render_footer(area: Rect, buf: &mut Buffer) {
    let mut spans = Vec::new();
    for (keys, label) in [
        (vec![KeyCode::Enter, KeyCode::Tab], " insert · "),
        (vec![KeyCode::Esc], " close · "),
        (vec![KeyCode::Up, KeyCode::Down], " select · "),
        (vec![KeyCode::Left, KeyCode::Right], " filter"),
    ] {
        let keys = keys
            .into_iter()
            .map(|key| {
                ratatui::text::Span::from(key_hint::plain(key))
                    .content
                    .into_owned()
            })
            .collect::<Vec<_>>()
            .join("/");
        spans.push(keys.bold());
        spans.push(label.dim());
    }
    let line = Line::from(spans);
    truncate_line_with_ellipsis_if_overflow(line, usize::from(area.width)).render(area, buf);
}
