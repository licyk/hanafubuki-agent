// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Highlighting an open code fence a line at a time while it streams
//! (Codex's `render/highlight_streaming.rs`): only syntect's parser state
//! is kept, under the ordinary limits, so each complete line is highlighted
//! once.

use ratatui::text::Line;
use syntect::easy::HighlightLines;
use syntect::highlighting::HighlightState;
use syntect::parsing::ParseState;
use syntect::util::LinesWithEndings;

use super::{
    exceeds_highlight_limits, find_syntax, line_spans, syntax_set, syntax_theme_revision,
    theme_lock, MAX_HIGHLIGHT_LINE_BYTES,
};

/// No state: the whole fence is plain text, its language unknown or a limit
/// crossed. The caller keeps the rows rendered so far.
pub(crate) struct StreamingCodeHighlighter {
    state: Option<HighlightedCode>,
}

struct HighlightedCode {
    bytes: usize,
    lines: usize,
    theme_revision: u64,
    syntax: (HighlightState, ParseState),
}

impl StreamingCodeHighlighter {
    /// The state after `code`, which the whole render already showed. `None`
    /// when the theme changed meanwhile, or `code` cannot be highlighted
    /// again: the caller renders the fence whole.
    pub(crate) fn new(code: &str, lang: &str, theme_revision: u64) -> Option<Self> {
        let theme = theme_lock();
        let theme = theme.read();
        if theme_revision != syntax_theme_revision() {
            return None;
        }
        let lines = code.lines().count();
        let syntax = find_syntax(lang).filter(|_| {
            !exceeds_highlight_limits(code.len(), lines)
                && !code
                    .lines()
                    .any(|line| line.len() > MAX_HIGHLIGHT_LINE_BYTES)
        });
        let Some(syntax) = syntax else {
            return Some(Self { state: None });
        };
        let mut highlighter = HighlightLines::new(syntax, &theme);
        for line in LinesWithEndings::from(code) {
            highlighter.highlight_line(line, syntax_set()).ok()?;
        }
        Some(Self {
            state: Some(HighlightedCode {
                bytes: code.len(),
                lines,
                theme_revision,
                syntax: highlighter.state(),
            }),
        })
    }

    /// The rows of `appended`, complete lines only. `None` drops the state:
    /// the caller renders the whole fence again, which also takes the
    /// colours off once the fence crosses a limit.
    pub(crate) fn append(mut self, appended: &str) -> Option<(Self, Vec<Line<'static>>)> {
        if !appended.ends_with('\n') {
            return None;
        }
        let Some(mut state) = self.state.take() else {
            let lines = appended
                .lines()
                .map(|line| Line::from(line.to_string()))
                .collect();
            return Some((self, lines));
        };
        let bytes = state.bytes.checked_add(appended.len())?;
        let lines = state.lines.checked_add(appended.lines().count())?;
        if exceeds_highlight_limits(bytes, lines)
            || appended
                .lines()
                .any(|line| line.len() > MAX_HIGHLIGHT_LINE_BYTES)
        {
            return None;
        }
        let theme = theme_lock();
        let theme = theme.read();
        if state.theme_revision != syntax_theme_revision() {
            return None;
        }
        let (highlight_state, parse_state) = state.syntax;
        let mut highlighter = HighlightLines::from_state(&theme, highlight_state, parse_state);
        let mut rendered = Vec::with_capacity(lines - state.lines);
        for line in LinesWithEndings::from(appended) {
            let ranges = highlighter.highlight_line(line, syntax_set()).ok()?;
            rendered.push(Line::from(line_spans(ranges)));
        }
        state.bytes = bytes;
        state.lines = lines;
        state.syntax = highlighter.state();
        self.state = Some(state);
        Some((self, rendered))
    }
}

#[cfg(test)]
mod tests;
