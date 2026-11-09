// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Single-line editing strips line breaks and keeps a grapheme-aligned cursor viewport.

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::TextArea;

impl TextArea {
    pub(crate) fn new_single_line() -> Self {
        Self {
            single_line: true,
            ..Self::new()
        }
    }

    pub(super) fn filter_line_breaks<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.single_line && text.contains(['\r', '\n']) {
            Cow::Owned(text.replace(['\r', '\n'], ""))
        } else {
            Cow::Borrowed(text)
        }
    }

    /// Where the visible part of the line starts when `width` columns show
    /// it with the cursor in view, and the cursor's column in it.
    pub(crate) fn single_line_viewport(&self, width: u16) -> (usize, u16) {
        let cursor_width = self.text[self.cursor_pos..]
            .graphemes(true)
            .next()
            .map_or(1, UnicodeWidthStr::width)
            .max(1)
            .min(usize::from(width));
        let mut start = self.cursor_pos;
        let mut column = 0;
        for (offset, grapheme) in self.text[..self.cursor_pos].grapheme_indices(true).rev() {
            let grapheme_width = grapheme.width();
            if column + grapheme_width > usize::from(width).saturating_sub(cursor_width) {
                break;
            }
            start = offset;
            column += grapheme_width;
        }
        (start, column as u16)
    }
}
