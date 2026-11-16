// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

pub(crate) use crate::style::selection_style;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Style, Stylize},
    text::Line,
    widgets::Widget,
};

#[derive(Default)]
pub(super) struct RenderedRows {
    pub lines: u16,
    pub has_above: bool,
    pub has_below: bool,
}

pub(super) fn active_tab_style() -> Style {
    Style::default()
        .fg(ratatui::style::Color::Reset)
        .bg(ratatui::style::Color::Reset)
        .bold()
        .not_dim()
        .underlined()
}

pub(super) fn render_scroll_indicators(area: Rect, buf: &mut Buffer, rendered: RenderedRows) {
    if rendered.lines == 0 || area.height < 3 || area.width == 0 {
        return;
    }
    for (visible, symbol, y) in [
        (rendered.has_above, "↑", area.y),
        (rendered.has_below, "↓", area.bottom() - 1),
    ] {
        if visible {
            Line::from(symbol)
                .fg(crate::style::accent_color())
                .render(Rect::new(area.x, y, 1, 1), buf);
        }
    }
}
