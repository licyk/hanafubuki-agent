// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use ratatui::text::Line;
pub(crate) fn append_markdown(
    markdown_source: &str,
    width: Option<usize>,
    lines: &mut Vec<Line<'static>>,
) {
    let rendered = crate::markdown_render::render_markdown_text_with_width(markdown_source, width);
    crate::render::line_utils::push_owned_lines(&rendered.lines, lines);
}

#[cfg(test)]
mod tests;
