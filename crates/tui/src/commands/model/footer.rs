//! What a model list shows under its rows, `/model`'s and the Provider
//! page's alike: while something is listed, the moving loading line, else
//! what was said in its place; then the notes (the selected model's facts
//! with their sources, the keys), wrapped to the width.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Widget;

use crate::bottom_pane::LoadingLine;
use crate::render::renderable::Renderable;
use crate::wrapping::word_wrap_lines;

/// What the line under the list is.
pub(crate) enum Status {
    /// Something is still listed: the moving loading line.
    Loading(String),
    /// Nothing more will be.
    Said(String),
}

/// The selected row's detail, when it has one, and the keys.
pub(crate) fn notes(detail: Option<String>, hint: String) -> Vec<Line<'static>> {
    detail
        .map(Line::from)
        .into_iter()
        .chain([Line::from(hint.dim())])
        .collect()
}

/// The lines under the list at `width`: the status, then the notes,
/// wrapped. The loading line is built only when `draw`ing, since building
/// it asks for the next frame; measuring counts a blank line in its place.
pub(crate) fn footer(
    loading: &LoadingLine,
    status: Option<Status>,
    mut notes: Vec<Line<'static>>,
    width: u16,
    draw: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match status {
        Some(Status::Loading(text)) if draw => lines.push(loading.line(&text)),
        Some(Status::Loading(_)) => lines.push(Line::default()),
        Some(Status::Said(text)) => notes.insert(0, Line::from(text)),
        None => {}
    }
    let width = usize::from(width.saturating_sub(2).max(1));
    lines.extend(
        notes
            .into_iter()
            .flat_map(|line| word_wrap_lines([line], width)),
    );
    lines
}

/// `body`, with `footer` on the lines under it.
pub(crate) fn render_footed(
    body: &dyn Renderable,
    footer: Vec<Line<'static>>,
    area: Rect,
    buf: &mut Buffer,
) {
    if area.is_empty() {
        return;
    }
    let body_area = Rect {
        height: area.height.saturating_sub(footer.len() as u16),
        ..area
    };
    body.render(body_area, buf);
    for (y, line) in (body_area.bottom()..area.bottom()).zip(footer) {
        line.render(
            Rect::new(area.x + 2, y, area.width.saturating_sub(2), 1),
            buf,
        );
    }
}
