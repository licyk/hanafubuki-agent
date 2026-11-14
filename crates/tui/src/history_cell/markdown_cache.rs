// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The last render of a Markdown cell, for one width and list spacing, so
//! a long transcript is not rendered again on every frame or layout cache
//! miss; another width renders again and replaces it. The rows keep their
//! links.

use crate::markdown_render::ListSpacing;
use crate::terminal_hyperlinks::HyperlinkLine;

#[derive(Debug, Default)]
pub(super) struct MarkdownRenderCache {
    cached: parking_lot::Mutex<Option<(MarkdownRenderKey, Vec<HyperlinkLine>)>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MarkdownRenderKey {
    width: u16,
    list_spacing: ListSpacing,
}

impl MarkdownRenderCache {
    /// The lines cached for `width` and `list_spacing`, else `render`'s.
    pub(super) fn render(
        &self,
        width: u16,
        list_spacing: ListSpacing,
        render: impl FnOnce() -> Vec<HyperlinkLine>,
    ) -> Vec<HyperlinkLine> {
        let key = MarkdownRenderKey {
            width,
            list_spacing,
        };
        if let Some((_, lines)) = self.cached.lock().as_ref().filter(|(at, _)| *at == key) {
            return lines.clone();
        }
        // Rendered outside the lock: a panic while rendering leaves the
        // cache as it was.
        let lines = render();
        *self.cached.lock() = Some((key, lines.clone()));
        lines
    }

    #[cfg(test)]
    pub(super) fn replace_cached_lines(&self, lines: Vec<HyperlinkLine>) {
        if let Some((_, cached)) = self.cached.lock().as_mut() {
            *cached = lines;
        }
    }
}
