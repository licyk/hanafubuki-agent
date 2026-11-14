// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The line still arriving, previewed: rendered after the stream's rows
//! and dropped when its newline commits it, so a preview never enters the
//! scrollback and the line is rendered from its source again. A preview is
//! bounded to the line's last 8 KiB; a link's destination waits until its
//! `)` arrives, its label shown meanwhile. A line whose start suggests a
//! block (indented, quoted, a fence, a table's pipe) is not previewed past
//! that point: its block is only known with its newline. Inside a display
//! formula still open the line is its source, wrapped, its TeX never read
//! as Markdown.

use ratatui::text::Line;

use crate::terminal_hyperlinks::HyperlinkLine;

use super::render::StreamRenderer;
use crate::markdown_render::ListSpacing;

const MAX_PREVIEW_BYTES: usize = 8192;

/// How the line is previewed: rendered as Markdown, or as a formula's
/// source.
pub(super) enum PreviewMode {
    Prose(StreamRenderer),
    Math,
}

/// The preview of one unterminated line, which only grows; reset when a
/// newline commits it.
#[derive(Default)]
pub(super) struct ProsePreview {
    pub(super) lines: Vec<HyperlinkLine>,
    scanned_len: usize,
    safe_len: usize,
    /// `safe_len`, before a link's destination still open.
    prose_safe_len: usize,
    has_pipe: bool,
    link: LinkPreview,
}

impl ProsePreview {
    /// The preview of `source`, the line so far: `true` when its rows
    /// changed.
    pub(super) fn update(
        &mut self,
        source: &str,
        width: Option<usize>,
        cwd: &str,
        mode: PreviewMode,
    ) -> bool {
        // Only the bytes that arrived since are scanned, however long the
        // line grows.
        self.has_pipe |= source[self.scanned_len..].contains('|');
        let advances = matches!(mode, PreviewMode::Math)
            || !(self.has_pipe
                || source.starts_with([' ', '\t', '>'])
                || source.starts_with("```")
                || source.starts_with("~~~")
                || matches!(source, "`" | "``" | "~" | "~~"));
        // A fence's first backticks may still turn out to be a code span.
        if advances || source.len() <= 2 {
            self.link.scan(source, self.scanned_len);
        }
        if advances {
            self.safe_len = source.len();
            self.prose_safe_len = source.len();
        }
        self.scanned_len = source.len();
        if let Some((start, _)) = self.link.destination {
            self.prose_safe_len = self.prose_safe_len.min(start);
        }
        let safe_len = match mode {
            PreviewMode::Prose(_) => self.prose_safe_len,
            PreviewMode::Math => self.safe_len,
        };
        let source = &source[..safe_len];
        let start = source.ceil_char_boundary(source.len().saturating_sub(MAX_PREVIEW_BYTES));
        let mut lines = match mode {
            PreviewMode::Prose(renderer) => {
                renderer(&source[start..], width, Some(cwd), ListSpacing::default()).lines
            }
            PreviewMode::Math => textwrap::wrap(&source[start..], width.unwrap_or(usize::MAX))
                .into_iter()
                .map(|line| Line::from(line.into_owned()).into())
                .collect(),
        };
        if start > 0 {
            lines.insert(0, Line::from("…").into());
        }
        if self.lines == lines {
            return false;
        }
        self.lines = lines;
        true
    }
}

/// Scans the line as it grows, without keeping a destination being read.
/// The label stays shown; its `(` and what follows wait for the `)`.
#[derive(Default)]
struct LinkPreview {
    brackets: usize,
    closed_label: bool,
    /// Where an open destination starts, and its parentheses' depth.
    destination: Option<(usize, usize)>,
    delimiter: Option<u8>,
    whitespace: bool,
    escaped: bool,
    code_ticks: usize,
    pending_ticks: usize,
}

impl LinkPreview {
    fn scan(&mut self, source: &str, start: usize) {
        for (index, byte) in source.bytes().enumerate().skip(start) {
            if self.destination.is_none() {
                if byte == b'`' && (self.code_ticks > 0 || !self.escaped) {
                    self.pending_ticks += 1;
                    self.closed_label = false;
                    continue;
                }
                if self.pending_ticks > 0 {
                    if self.code_ticks == 0 {
                        self.code_ticks = self.pending_ticks;
                    } else if self.code_ticks == self.pending_ticks {
                        self.code_ticks = 0;
                    }
                    self.pending_ticks = 0;
                }
                if self.code_ticks > 0 {
                    continue;
                }
            }
            if self.escaped {
                self.escaped = false;
                self.closed_label = false;
                self.whitespace = false;
                continue;
            }
            if byte == b'\\' {
                self.escaped = true;
                self.closed_label = false;
                continue;
            }
            if let Some((destination_start, depth)) = self.destination.as_mut() {
                if let Some(delimiter) = self.delimiter {
                    if byte == delimiter {
                        self.delimiter = None;
                    }
                } else if byte == b'<'
                    && *depth == 1
                    && (index == *destination_start + 1 || self.whitespace)
                {
                    self.delimiter = Some(b'>');
                } else if matches!(byte, b'\'' | b'"') && self.whitespace && *depth == 1 {
                    self.delimiter = Some(byte);
                } else if byte == b'(' {
                    *depth += 1;
                } else if byte == b')' {
                    *depth -= 1;
                    if *depth == 0 {
                        self.destination = None;
                    }
                }
                self.whitespace = byte.is_ascii_whitespace();
            } else {
                match byte {
                    b'[' => self.brackets += 1,
                    b']' if self.brackets > 0 => {
                        self.brackets -= 1;
                        self.closed_label = true;
                        continue;
                    }
                    b'(' if self.closed_label => {
                        self.destination = Some((index, 1));
                        self.whitespace = false;
                    }
                    _ => {}
                }
                self.closed_label = false;
            }
        }
    }
}
