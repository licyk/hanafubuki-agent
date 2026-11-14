// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The rows written: the line being built, wrapped under the quote and list
//! prefixes when it is done, and the table rows laid out already. A row
//! carries its web links beside its text.

use ratatui::text::{Line, Span};

use super::Writer;
use crate::terminal_hyperlinks::{remap_source_wrapped_line, HyperlinkLine};
use crate::wrapping::{adaptive_wrap_line_with_source, RtOptions};

pub(super) use crate::wrapping::display_width;

impl Writer<'_> {
    pub(super) fn flush_current_line(&mut self) {
        if let Some(line) = self.current_line_content.take() {
            let style = self.current_line_style;
            // Code is not wrapped, so its whitespace copies as it is.
            let wrap_width = self.wrap_width.filter(|_| !self.current_line_in_code_block);
            if let Some(width) = wrap_width {
                let opts = RtOptions::new(width)
                    .initial_indent(self.current_initial_indent.clone().into())
                    .subsequent_indent(self.current_subsequent_indent.clone().into());
                // A URL is kept whole, on a row of its own when it does
                // not fit, wider than the row when it is wider.
                let wrapped = adaptive_wrap_line_with_source(&line.line, opts);
                for row in remap_source_wrapped_line(&line, wrapped) {
                    self.lines.push(row.style(style));
                }
            } else {
                let indent = std::mem::take(&mut self.current_initial_indent);
                self.lines.push(line.prefixed(indent).style(style));
            }
            self.current_initial_indent.clear();
            self.current_subsequent_indent.clear();
            self.current_line_in_code_block = false;
            self.trailing_local_link = self.line_ends_with_local_link_target;
            self.line_ends_with_local_link_target = false;
        }
    }

    pub(super) fn is_blockquote_active(&self) -> bool {
        self.indent_stack
            .iter()
            .any(|ctx| ctx.prefix.iter().any(|span| span.content.contains('>')))
    }

    /// A row laid out at its width already (a table's): the quote and list
    /// prefixes go before it, and it is not wrapped again.
    pub(super) fn push_prewrapped_line(&mut self, line: HyperlinkLine, pending_marker_line: bool) {
        self.flush_current_line();
        let style = if self.is_blockquote_active() {
            self.styles.blockquote.patch(line.line.style)
        } else {
            line.line.style
        };
        let prefix = self.prefix_spans(pending_marker_line);
        self.lines.push(line.prefixed(prefix).style(style));
    }

    pub(super) fn push_line(&mut self, line: impl Into<HyperlinkLine>) {
        let line = line.into();
        self.flush_current_line();
        let style = if self.is_blockquote_active() {
            self.styles.blockquote
        } else {
            line.line.style
        };
        let was_pending = self.pending_marker_line;

        self.current_initial_indent = self.prefix_spans(was_pending);
        self.current_subsequent_indent = self.prefix_spans(false);
        self.current_line_style = style;
        self.current_line_content = Some(line);
        self.current_line_in_code_block = self.in_code_block;
        self.line_ends_with_local_link_target = false;

        self.pending_marker_line = false;
    }

    /// `span` on the line being built, linked when it is a web link's.
    pub(super) fn push_span(&mut self, span: Span<'static>) {
        let span = self.style_link_label(span);
        let destination = self.web_link_destination();
        if self.current_line_content.is_none() {
            self.push_line(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            line.push_span(span, destination.as_deref());
        }
    }

    /// A run of text: a web link's label, or prose whose bare web URLs are
    /// styled and linked.
    pub(super) fn push_text_span(&mut self, span: Span<'static>) {
        if self.link.is_some() || self.in_code_block {
            self.push_span(span);
            return;
        }
        let annotated = self.style_bare_web_urls(span);
        if self.current_line_content.is_none() {
            self.push_line(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            line.append(annotated);
        }
    }

    pub(super) fn push_blank_line(&mut self) {
        self.flush_current_line();
        if self.indent_stack.iter().all(|ctx| ctx.is_list) {
            self.lines.push(HyperlinkLine::default());
        } else {
            self.push_line(Line::default());
            self.flush_current_line();
        }
    }

    pub(super) fn prefix_spans(&self, pending_marker_line: bool) -> Vec<Span<'static>> {
        let mut prefix: Vec<Span<'static>> = Vec::new();
        let last_marker_index = if pending_marker_line {
            self.indent_stack
                .iter()
                .rposition(|ctx| ctx.marker.is_some())
        } else {
            None
        };
        let last_list_index = self.indent_stack.iter().rposition(|ctx| ctx.is_list);

        for (i, ctx) in self.indent_stack.iter().enumerate() {
            if pending_marker_line {
                if let Some(marker) = ctx.marker.as_ref().filter(|_| Some(i) == last_marker_index) {
                    prefix.extend(marker.iter().cloned());
                    continue;
                }
                if ctx.is_list && last_marker_index.is_some_and(|idx| idx > i) {
                    continue;
                }
            } else if ctx.is_list && Some(i) != last_list_index {
                continue;
            }
            prefix.extend(ctx.prefix.iter().cloned());
        }

        prefix
    }
}

/// The columns `spans` take in a terminal.
pub(super) fn spans_display_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| display_width(&span.content)).sum()
}
