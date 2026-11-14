// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The `/diff` overlay's lines: `git diff`'s unified diff with its headers
//! in bold, its chunk headers in cyan, and each file's lines highlighted
//! by the file's language when its name tells it.

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};

use super::highlight_lines;

pub(crate) fn git_diff_lines(text: &str) -> Vec<Line<'static>> {
    if text.trim().is_empty() {
        return vec!["No changes detected.".italic().into()];
    }
    let mut out = Vec::new();
    let mut file = FileSection::default();
    for line in text.lines() {
        if line.starts_with("diff ") {
            file.flush(&mut out);
        }
        match line_kind(line) {
            Some(sign) if !file.in_header => file.code.push((sign, line[1..].to_string())),
            _ => {
                file.flush_code(&mut out);
                file.header(line, &mut out);
            }
        }
    }
    file.flush(&mut out);
    out
}

/// The sign of a line of code in a chunk.
fn line_kind(line: &str) -> Option<char> {
    line.chars()
        .next()
        .filter(|sign| matches!(sign, '+' | '-' | ' '))
}

/// The file being read: its path once its headers named it, and its
/// lines of code since the last header.
#[derive(Default)]
struct FileSection {
    path: Option<String>,
    /// Between `diff --git` and the first chunk, where `---` and `+++`
    /// name the files rather than remove and add lines.
    in_header: bool,
    code: Vec<(char, String)>,
}

impl FileSection {
    fn header(&mut self, line: &str, out: &mut Vec<Line<'static>>) {
        let styled: Line<'static> = if line.starts_with("diff ") {
            self.in_header = true;
            self.path = line.rsplit_once(" b/").map(|(_, path)| path.to_string());
            line.to_string().bold().into()
        } else if line.starts_with("@@") {
            self.in_header = false;
            line.to_string().cyan().into()
        } else if self.in_header {
            if let Some(path) = line.strip_prefix("+++ b/") {
                self.path = Some(path.to_string());
            }
            line.to_string().bold().into()
        } else {
            line.to_string().dim().into()
        };
        out.push(styled);
    }

    fn flush_code(&mut self, out: &mut Vec<Line<'static>>) {
        if self.code.is_empty() {
            return;
        }
        let texts: Vec<String> = self.code.iter().map(|(_, text)| text.clone()).collect();
        let highlighted = self
            .path
            .as_deref()
            .and_then(|path| highlight_lines(path, &texts));
        let mut highlighted = highlighted.into_iter().flatten();
        for (sign, text) in std::mem::take(&mut self.code) {
            let style = match sign {
                '+' => Style::default().fg(Color::Green),
                '-' => Style::default().fg(Color::Red),
                _ => Default::default(),
            };
            let mut spans = vec![Span::styled(sign.to_string(), style)];
            match highlighted.next() {
                Some(code) if sign == '-' => spans.extend(
                    code.into_iter()
                        .map(|span| span.add_modifier(Modifier::DIM)),
                ),
                Some(code) => spans.extend(code),
                None => spans.push(Span::styled(text, style)),
            }
            out.push(Line::from(spans));
        }
    }

    fn flush(&mut self, out: &mut Vec<Line<'static>>) {
        self.flush_code(out);
        *self = Self::default();
    }
}
