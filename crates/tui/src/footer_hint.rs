// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use std::ops::Range;

use ratatui::{style::Stylize, text::Line};

pub(crate) fn shortcut(keys: &str, label: &str) -> Line<'static> {
    Line::from(vec![keys.to_owned().bold(), format!(" {label}").dim()])
}

/// Measured columns keep pointer targets in step with the hints that fit.
pub(crate) struct HintLine {
    pub(crate) line: Line<'static>,
    pub(crate) items: Vec<Range<u16>>,
}

impl HintLine {
    pub(crate) fn new(hints: impl IntoIterator<Item = Line<'static>>, width: u16) -> Self {
        let mut hints = hints.into_iter().peekable();
        let mut line = Line::default();
        let mut items = Vec::new();
        let mut end = 0;
        while let Some(hint) = hints.next() {
            let start = end + if items.is_empty() { 0 } else { 3 };
            let next = start + hint.width();
            if next + usize::from(hints.peek().is_some()) * 2 > usize::from(width) {
                if width > 0 {
                    line.push_span(if items.is_empty() { "…" } else { " …" }.dim());
                }
                break;
            }
            if !items.is_empty() {
                line.push_span(" · ".dim());
            }
            line.extend(hint.spans);
            items.push(start as u16..next as u16);
            end = next;
        }
        Self { line, items }
    }
}
