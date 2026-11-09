// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A patch as the transcript shows it: a header with the files and their
//! line counts, then each file's lines with a sign column. Codex reads a
//! unified diff of the changed file and numbers its lines; the Agent's
//! reading of a patch (`PatchPreview`, the parser that applies it) has no
//! line numbers, so this has no gutter, and a chunk header is a `⋮` row.
//!
//! A file whose language its name tells is highlighted with the syntax
//! theme: its lines are read as one piece of code, the sign keeps the
//! diff's colour, and a removed line is dimmed. `git_diff` is the same for
//! the `/diff` overlay's unified diff.

mod git_diff;

pub(crate) use git_diff::git_diff_lines;

use hanafubuki_agent::{
    PatchPreview, PatchPreviewFile, PatchPreviewLineKind, PatchPreviewOperation,
};
use std::path::Path;

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line as RtLine, Span as RtSpan};

use crate::render::highlight::{exceeds_highlight_limits, highlight_code_to_styled_spans};
use crate::render::line_utils::prefix_lines;
use crate::text_formatting::strip_terminal_controls;

pub(crate) fn create_diff_summary(
    preview: &PatchPreview,
    wrap_cols: usize,
) -> Vec<RtLine<'static>> {
    if let Some(error) = &preview.error {
        return vec![
            RtLine::from(vec!["  └ ".dim(), "the patch could not be read: ".dim()]),
            RtLine::from(vec!["    ".into(), error.clone().red()]),
        ];
    }
    let mut out: Vec<RtLine<'static>> = Vec::new();
    let files = &preview.files;
    let total_added: usize = files.iter().map(|file| file.added).sum();
    let total_removed: usize = files.iter().map(|file| file.removed).sum();
    let mut header: Vec<RtSpan<'static>> = vec!["  └ ".dim()];
    if let [file] = &files[..] {
        let verb = match file.operation {
            PatchPreviewOperation::Add => "Added",
            PatchPreviewOperation::Delete => "Deleted",
            PatchPreviewOperation::Update => "Edited",
        };
        header.push(verb.bold());
        header.push(" ".into());
        header.extend(render_path(file));
        header.push(" ".into());
        header.extend(render_line_count_summary(file.added, file.removed));
    } else {
        let noun = if files.len() == 1 { "file" } else { "files" };
        header.push("Edited".bold());
        header.push(format!(" {} {noun} ", files.len()).into());
        header.extend(render_line_count_summary(total_added, total_removed));
    }
    out.push(RtLine::from(header));

    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            out.push("".into());
        }
        if files.len() > 1 {
            let mut header: Vec<RtSpan<'static>> = vec!["    ".into()];
            header.extend(render_path(file));
            header.push(" ".into());
            header.extend(render_line_count_summary(file.added, file.removed));
            out.push(RtLine::from(header));
        }
        let mut lines = Vec::new();
        render_file(file, &mut lines, wrap_cols.saturating_sub(4));
        out.extend(prefix_lines(lines, "    ".into(), "    ".into()));
    }
    out
}

fn render_path(file: &PatchPreviewFile) -> Vec<RtSpan<'static>> {
    let mut spans = vec![RtSpan::from(strip_terminal_controls(&file.path))];
    if let Some(moved_to) = &file.moved_to {
        spans.push(format!(" → {}", strip_terminal_controls(moved_to)).into());
    }
    spans
}

fn render_line_count_summary(added: usize, removed: usize) -> Vec<RtSpan<'static>> {
    vec![
        "(".into(),
        format!("+{added}").green(),
        " ".into(),
        format!("-{removed}").red(),
        ")".into(),
    ]
}

fn render_file(file: &PatchPreviewFile, out: &mut Vec<RtLine<'static>>, width: usize) {
    let path = file.moved_to.as_deref().unwrap_or(&file.path);
    let texts: Vec<String> = file
        .lines
        .iter()
        .filter(|line| is_code(line.kind))
        .map(|line| strip_terminal_controls(&line.text))
        .collect();
    let mut highlighted = highlight_lines(path, &texts).into_iter().flatten();
    let mut texts = texts.iter();
    let mut seen_content = false;
    for line in &file.lines {
        let kind = match line.kind {
            PatchPreviewLineKind::Added => DiffLineKind::Added,
            PatchPreviewLineKind::Removed => DiffLineKind::Removed,
            PatchPreviewLineKind::Context => DiffLineKind::Context,
            // The first chunk needs no separator from what is above it.
            PatchPreviewLineKind::Hunk if !seen_content => continue,
            PatchPreviewLineKind::Hunk => {
                out.push(RtLine::from("⋮".dim()));
                continue;
            }
            PatchPreviewLineKind::Marker => {
                seen_content = true;
                out.push(RtLine::from(strip_terminal_controls(&line.text)).dim());
                continue;
            }
        };
        seen_content = true;
        let text = texts.next().map(String::as_str).unwrap_or_default();
        out.extend(push_wrapped_diff_line(
            kind,
            text,
            highlighted.next(),
            width,
        ));
    }
    if file.omitted_lines > 0 {
        out.push(RtLine::from(format!("… +{} lines", file.omitted_lines)).dim());
    }
}

fn is_code(kind: PatchPreviewLineKind) -> bool {
    matches!(
        kind,
        PatchPreviewLineKind::Added | PatchPreviewLineKind::Removed | PatchPreviewLineKind::Context
    )
}

/// The lines of one diff row's content, by what the row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
    Added,
    Removed,
    Context,
}

impl DiffLineKind {
    fn sign(self) -> char {
        match self {
            DiffLineKind::Added => '+',
            DiffLineKind::Removed => '-',
            DiffLineKind::Context => ' ',
        }
    }

    /// The colour of an unhighlighted row.
    fn style(self) -> Style {
        match self {
            DiffLineKind::Added => style_add(),
            DiffLineKind::Removed => style_del(),
            DiffLineKind::Context => style_context(),
        }
    }
}

/// `lines` of the file at `path` highlighted as one piece of code, when
/// its name tells its language and it is not too large.
pub(crate) fn highlight_lines(path: &str, lines: &[String]) -> Option<Vec<Vec<RtSpan<'static>>>> {
    let total_bytes = lines.iter().map(|line| line.len() + 1).sum();
    if lines.is_empty() || exceeds_highlight_limits(total_bytes, lines.len()) {
        return None;
    }
    let path = Path::new(path);
    let lang = path
        .extension()
        .or_else(|| path.file_name())
        .and_then(|lang| lang.to_str())?;
    let highlighted = highlight_code_to_styled_spans(&lines.join("\n"), lang)?;
    (highlighted.len() == lines.len()).then_some(highlighted)
}

/// One diff row, wrapped to `width` with the sign column kept: the sign in
/// the row's colour, the content highlighted when `highlighted` has it.
pub(crate) fn push_wrapped_diff_line(
    kind: DiffLineKind,
    text: &str,
    highlighted: Option<Vec<RtSpan<'static>>>,
    width: usize,
) -> Vec<RtLine<'static>> {
    let content = match highlighted {
        Some(spans) => {
            let dim = kind == DiffLineKind::Removed;
            spans
                .into_iter()
                .map(|span| {
                    if dim {
                        span.add_modifier(Modifier::DIM)
                    } else {
                        span
                    }
                })
                .collect()
        }
        None => vec![RtSpan::styled(text.to_string(), kind.style())],
    };
    // Split at a character boundary so each row's chunk fits the columns
    // left after the sign.
    let columns = width.saturating_sub(1).max(1);
    let mut rows: Vec<RtLine<'static>> = Vec::new();
    let mut row: Vec<RtSpan<'static>> = vec![RtSpan::styled(kind.sign().to_string(), kind.style())];
    let mut used = 0;
    for span in content {
        let mut rest: &str = &span.content;
        while !rest.is_empty() {
            if used == columns {
                rows.push(RtLine::from(std::mem::take(&mut row)));
                // Continuation rows keep a space for the sign column.
                row.push(RtSpan::raw(" "));
                used = 0;
            }
            let split = rest
                .char_indices()
                .nth(columns - used)
                .map_or(rest.len(), |(index, _)| index);
            let (chunk, tail) = rest.split_at(split);
            used += chunk.chars().count();
            row.push(RtSpan::styled(chunk.to_string(), span.style));
            rest = tail;
        }
    }
    rows.push(RtLine::from(row));
    rows
}

fn style_context() -> Style {
    Style::default()
}

pub(crate) fn style_add() -> Style {
    Style::default().fg(Color::Green)
}

pub(crate) fn style_del() -> Style {
    Style::default().fg(Color::Red)
}

#[cfg(test)]
mod tests;
