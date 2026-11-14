// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A stream's committed source rendered incrementally: the top-level blocks
//! before the last are rendered once and kept; only the last, which the
//! next line may still change (a list's spacing, a setext heading, a fence,
//! a table), is rendered again as lines arrive.

use crate::terminal_hyperlinks::HyperlinkLine;

use super::code_fence::OpenCodeFence;
use crate::markdown_render::{ListSpacing, StreamingMarkdownRender};
use crate::render::highlight::syntax_theme_revision;

/// How a stream's Markdown is rendered: an answer's (its table fences
/// unwrapped) or a plan's.
pub(crate) type StreamRenderer =
    fn(&str, Option<usize>, Option<&str>, ListSpacing) -> StreamingMarkdownRender;

/// The render of the committed source, split where the kept prefix ends,
/// in the source and in the rows.
pub(super) struct StreamingRender {
    pub(super) renderer: StreamRenderer,
    pub(super) list_spacing: ListSpacing,
    pub(super) lines: Vec<HyperlinkLine>,
    pub(super) mutable_fence_start: Option<usize>,
    /// The last row ends with a local file link's target.
    pub(super) trailing_local_link: bool,
    /// Where the last top-level block starts, also after a full render.
    pub(super) completed_source_len: usize,
    /// The source of the complete top-level blocks kept.
    stable_source_len: usize,
    /// Their rows.
    stable_rendered_len: usize,
    /// A reference definition can change any block: every line renders
    /// the whole source again.
    pub(super) has_reference_link_definition: bool,
    /// The last block is a code fence still open, extended a line at a
    /// time.
    open_code_fence: Option<OpenCodeFence>,
}

impl StreamingRender {
    pub(super) fn new(renderer: StreamRenderer, list_spacing: ListSpacing) -> Self {
        Self {
            renderer,
            list_spacing,
            lines: Vec::new(),
            mutable_fence_start: None,
            trailing_local_link: false,
            completed_source_len: 0,
            stable_source_len: 0,
            stable_rendered_len: 0,
            has_reference_link_definition: false,
            open_code_fence: None,
        }
    }

    /// `source` rendered whole, at `width`.
    pub(super) fn render_source(
        &self,
        source: &str,
        width: Option<usize>,
        cwd: &str,
    ) -> StreamingMarkdownRender {
        (self.renderer)(source, width, Some(cwd), self.list_spacing)
    }

    /// The whole source rendered again, nothing kept: at a new width, and
    /// while a reference definition can change any block.
    pub(super) fn recompute(&mut self, source: &str, width: Option<usize>, cwd: &str) {
        let rendered = self.render_source(source, width, cwd);
        self.open_code_fence = None;
        self.has_reference_link_definition = rendered.has_reference_link_definition;
        self.completed_source_len = rendered.last_top_level_block_start.unwrap_or(0);
        self.mutable_fence_start = rendered.mutable_fence_start;
        self.trailing_local_link = rendered.trailing_local_link;
        self.lines = rendered.lines;
        self.stable_source_len = 0;
        self.stable_rendered_len = 0;
    }

    /// `committed_source` (the end of `raw_source`) added: the kept blocks
    /// stay, the last block is rendered again, and the blocks it completed
    /// are kept from now on.
    pub(super) fn append(
        &mut self,
        raw_source: &str,
        committed_source: &str,
        width: Option<usize>,
        cwd: &str,
    ) {
        if self.has_reference_link_definition {
            self.recompute(raw_source, width, cwd);
            return;
        }
        if let Some(fence) = self.open_code_fence.take() {
            if let Some((fence, lines)) = fence.append(raw_source, committed_source) {
                self.lines.extend(lines);
                self.open_code_fence = Some(fence);
                return;
            }
        }

        let pending_source = &raw_source[self.stable_source_len..];
        let theme_revision = syntax_theme_revision();
        let pending = self.render_source(pending_source, width, cwd);
        if pending.has_reference_link_definition {
            self.recompute(raw_source, width, cwd);
            return;
        }
        self.mutable_fence_start = pending
            .mutable_fence_start
            .map(|start| self.stable_source_len + start);
        self.trailing_local_link = pending.trailing_local_link;

        let final_block_start = pending.last_top_level_block_start.unwrap_or(0);
        self.completed_source_len = self.stable_source_len + final_block_start;
        self.open_code_fence = OpenCodeFence::detect(
            &pending_source[final_block_start..],
            raw_source.len(),
            theme_revision,
        );

        let newly_stable_rendered_len = pending.last_top_level_block_start.map(|boundary| {
            let newly_stable = self.render_source(&pending_source[..boundary], width, cwd);
            self.stable_source_len += boundary;
            newly_stable.lines.len()
        });

        self.lines.truncate(self.stable_rendered_len);
        // The kept blocks and the rest are separated as in a whole render.
        if !self.lines.is_empty()
            && (!pending.lines.is_empty() || !pending_source.trim().is_empty())
            && !pending.first_top_level_block_is_html
        {
            self.lines.push(HyperlinkLine::default());
        }
        let pending_render_start = self.lines.len();
        self.lines.extend(pending.lines);
        if let Some(newly_stable_rendered_len) = newly_stable_rendered_len {
            self.stable_rendered_len = pending_render_start + newly_stable_rendered_len;
        }
    }
}

#[cfg(test)]
mod tests;
