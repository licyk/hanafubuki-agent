// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A cautious fast path for one open top-level code fence that names its
//! language: its next complete lines are highlighted and appended to the
//! rows, instead of the whole fence being rendered again at each line.
//! Anything the whole render might treat otherwise goes back to it.

use ratatui::text::Span;

use crate::terminal_hyperlinks::HyperlinkLine;

use crate::render::highlight::{
    syntax_theme_revision, StreamingCodeHighlighter, MAX_HIGHLIGHT_LINE_BYTES,
};

/// A fence whose source is the code the parser reads, as it grows.
pub(super) struct OpenCodeFence {
    marker: u8,
    marker_len: usize,
    language: String,
    content_start: usize,
    source_len: usize,
    theme_revision: u64,
    /// Made when a second chunk arrives, so a fence that came whole is not
    /// highlighted twice.
    highlighter: Option<StreamingCodeHighlighter>,
}

impl OpenCodeFence {
    /// The last top-level block, `source`, when it is such a fence, after
    /// its whole render; `source_len` is the committed source's length,
    /// which the fence ends at.
    ///
    /// Indentation, `\r` and NUL, escaped info strings, a Markdown fence
    /// (unwrapped or shown as written once it closes), and any line that
    /// might close the fence stay with the whole render, so this need not
    /// repeat CommonMark's rules for closing a fence.
    pub(super) fn detect(source: &str, source_len: usize, theme_revision: u64) -> Option<Self> {
        let marker = *source.as_bytes().first()?;
        if marker != b'`' && marker != b'~' {
            return None;
        }
        let (opening, code) = source.split_once('\n')?;
        let marker_len = opening.bytes().take_while(|byte| *byte == marker).count();
        if marker_len < 3
            || !source.ends_with('\n')
            || theme_revision != syntax_theme_revision()
            || source.contains(['\r', '\0'])
        {
            return None;
        }
        let info = opening[marker_len..].trim_matches([' ', '\t', '\u{b}', '\u{c}']);
        if info.contains(['&', '\\']) || (marker == b'`' && info.contains('`')) {
            return None;
        }
        let language = info
            .split([',', ' ', '\t'])
            .next()
            .filter(|language| !language.is_empty())?;
        if crate::table_detect::is_markdown_fence_info(info, 0)
            || language.len() > MAX_HIGHLIGHT_LINE_BYTES
            || has_possible_closing_line(code, marker, marker_len)
        {
            return None;
        }
        Some(Self {
            marker,
            marker_len,
            language: language.to_string(),
            content_start: source_len.checked_sub(code.len())?,
            source_len,
            theme_revision,
            highlighter: None,
        })
    }

    /// The rows of `committed_source`, when it extends this fence and
    /// nothing else; `None` gives the fence back to the whole render.
    pub(super) fn append(
        mut self,
        raw_source: &str,
        committed_source: &str,
    ) -> Option<(Self, Vec<HyperlinkLine>)> {
        if self.source_len.checked_add(committed_source.len()) != Some(raw_source.len())
            || !raw_source.ends_with(committed_source)
            || committed_source.contains(['\r', '\0'])
            || has_possible_closing_line(committed_source, self.marker, self.marker_len)
            || self.theme_revision != syntax_theme_revision()
        {
            return None;
        }
        let highlighter = match self.highlighter.take() {
            Some(highlighter) => highlighter,
            None => StreamingCodeHighlighter::new(
                &raw_source[self.content_start..self.source_len],
                &self.language,
                self.theme_revision,
            )?,
        };
        let (highlighter, lines) = highlighter.append(committed_source)?;
        self.highlighter = Some(highlighter);
        self.source_len = raw_source.len();
        let lines = lines
            .into_iter()
            .map(|mut line| {
                // The writer's indent of a top-level fence: an empty span.
                line.spans.insert(0, Span::default());
                HyperlinkLine::new(line)
            })
            .collect();
        Some((self, lines))
    }
}

/// Whether a line might close the fence; a false alarm only costs a whole
/// render.
fn has_possible_closing_line(source: &str, marker: u8, marker_len: usize) -> bool {
    source.lines().any(|line| {
        line.trim_start_matches([' ', '\t'])
            .bytes()
            .take_while(|byte| *byte == marker)
            .count()
            >= marker_len
    })
}

#[cfg(test)]
mod tests;
