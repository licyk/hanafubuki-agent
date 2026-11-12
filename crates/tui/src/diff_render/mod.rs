// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A patch as the transcript shows it: a header with the files and their
//! line counts, then each file's lines behind a sign column. Codex reads a
//! unified diff of the changed file and numbers its lines; the Agent's
//! reading of a patch (`AgentPatchPreview`, from the parser that applies
//! it) knows the numbers only of a file it adds, so only those rows have a
//! line-number gutter, and a chunk header is a `⋮` row.
//!
//! A file whose language its name tells is highlighted with the syntax
//! theme: its lines are read as one piece of code, the sign keeps the
//! diff's colour, and a removed line is dimmed. `git_diff` is the same for
//! the `/diff` overlay's unified diff.

mod git_diff;

pub(crate) use git_diff::git_diff_lines;

use hanafubuki_agent::{
    AgentPatchFile, AgentPatchLineKind, AgentPatchOperation, AgentPatchPreview,
};
use std::path::Path;

use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line as RtLine, Span as RtSpan};

use crate::render::highlight::{exceeds_highlight_limits, highlight_code_to_styled_spans};
use crate::render::line_utils::prefix_lines;
use crate::text_formatting::strip_terminal_controls;

/// The patch after the entry's bullet: `Edited src/lib.rs (+1 -1)`, or
/// `Edited 3 files (+20 -5)`; `failed` when it did not apply.
pub(crate) fn patch_title(preview: &AgentPatchPreview, failed: bool) -> Vec<RtSpan<'static>> {
    if preview.error.is_some() {
        return vec!["Could not read the patch".bold()];
    }
    let files = &preview.files;
    let mut title: Vec<RtSpan<'static>> = Vec::new();
    if let [file] = &files[..] {
        let (done, failed_verb) = match file.operation {
            AgentPatchOperation::Add => ("Added", "Failed to add"),
            AgentPatchOperation::Delete => ("Deleted", "Failed to delete"),
            AgentPatchOperation::Update => ("Edited", "Failed to edit"),
        };
        title.push(if failed { failed_verb } else { done }.bold());
        title.push(" ".into());
        title.extend(render_path(file));
        title.push(" ".into());
        title.extend(render_line_count_summary(file.added, file.removed));
    } else {
        let noun = if files.len() == 1 { "file" } else { "files" };
        title.push(if failed { "Failed to edit" } else { "Edited" }.bold());
        title.push(format!(" {} {noun} ", files.len()).into());
        title.extend(render_line_count_summary(
            files.iter().map(|file| file.added).sum(),
            files.iter().map(|file| file.removed).sum(),
        ));
    }
    title
}

/// A row per file of a patch of several, and why an unreadable patch could
/// not be read: what the patch shows collapsed.
pub(crate) fn patch_summary_rows(preview: &AgentPatchPreview) -> Vec<RtLine<'static>> {
    if let Some(error) = &preview.error {
        return vec![RtLine::from(strip_terminal_controls(error).red())];
    }
    if preview.files.len() < 2 {
        return Vec::new();
    }
    preview.files.iter().map(file_row).collect()
}

/// Each file's diff at `width` columns, under its row when the patch has
/// several files.
pub(crate) fn patch_diff_lines(preview: &AgentPatchPreview, width: usize) -> Vec<RtLine<'static>> {
    if preview.error.is_some() {
        return patch_summary_rows(preview);
    }
    let files = &preview.files;
    let mut out: Vec<RtLine<'static>> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            out.push("".into());
        }
        if files.len() > 1 {
            out.push(file_row(file));
            let mut lines = Vec::new();
            render_file(file, &mut lines, width.saturating_sub(2));
            out.extend(prefix_lines(lines, "  ".into(), "  ".into()));
        } else {
            render_file(file, &mut out, width);
        }
    }
    out
}

/// Whether the patch has rows of a diff to show.
pub(crate) fn patch_has_lines(preview: &AgentPatchPreview) -> bool {
    preview
        .files
        .iter()
        .any(|file| !file.lines.is_empty() || file.omitted_lines > 0)
}

fn file_row(file: &AgentPatchFile) -> RtLine<'static> {
    let mut row = render_path(file);
    row.push(" ".into());
    row.extend(render_line_count_summary(file.added, file.removed));
    RtLine::from(row)
}

fn render_path(file: &AgentPatchFile) -> Vec<RtSpan<'static>> {
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

fn render_file(file: &AgentPatchFile, out: &mut Vec<RtLine<'static>>, width: usize) {
    let path = file.moved_to.as_deref().unwrap_or(&file.path);
    // An added file's rows are its lines from the first; no other file's
    // numbers are known.
    let gutter =
        (file.operation == AgentPatchOperation::Add).then(|| line_number_width(file.added));
    let mut number = 0;
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
            AgentPatchLineKind::Added => DiffLineKind::Added,
            AgentPatchLineKind::Removed => DiffLineKind::Removed,
            AgentPatchLineKind::Context => DiffLineKind::Context,
            // The first chunk needs no separator from what is above it.
            AgentPatchLineKind::Hunk if !seen_content => continue,
            AgentPatchLineKind::Hunk => {
                out.push(RtLine::from("⋮".dim()));
                continue;
            }
            AgentPatchLineKind::Marker => {
                seen_content = true;
                out.push(RtLine::from(strip_terminal_controls(&line.text)).dim());
                continue;
            }
        };
        seen_content = true;
        number += 1;
        let text = texts.next().map(String::as_str).unwrap_or_default();
        out.extend(push_wrapped_diff_line(
            kind,
            text,
            highlighted.next(),
            gutter.map(|width| (number, width)),
            width,
        ));
    }
    if file.omitted_lines > 0 {
        out.push(RtLine::from(format!("… +{} lines", file.omitted_lines)).dim());
    }
}

fn is_code(kind: AgentPatchLineKind) -> bool {
    matches!(
        kind,
        AgentPatchLineKind::Added | AgentPatchLineKind::Removed | AgentPatchLineKind::Context
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

/// The columns of a right-aligned line number up to `max`.
pub(crate) fn line_number_width(max: usize) -> usize {
    max.max(1).to_string().len()
}

/// One diff row, wrapped to `width` with the sign column kept: the sign in
/// the row's colour, the content highlighted when `highlighted` has it,
/// behind its line number right-aligned in `gutter.1` columns when it has
/// one.
pub(crate) fn push_wrapped_diff_line(
    kind: DiffLineKind,
    text: &str,
    highlighted: Option<Vec<RtSpan<'static>>>,
    gutter: Option<(usize, usize)>,
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
    let gutter_columns = gutter.map_or(0, |(_, columns)| columns + 1);
    // Split at a character boundary so each row's chunk fits the columns
    // left after the gutter and the sign.
    let columns = width.saturating_sub(gutter_columns + 1).max(1);
    let mut rows: Vec<RtLine<'static>> = Vec::new();
    let mut row: Vec<RtSpan<'static>> = Vec::new();
    if let Some((number, columns)) = gutter {
        row.push(format!("{number:>columns$} ").dim());
    }
    row.push(RtSpan::styled(kind.sign().to_string(), kind.style()));
    let mut used = 0;
    for span in content {
        let mut rest: &str = &span.content;
        while !rest.is_empty() {
            if used == columns {
                rows.push(RtLine::from(std::mem::take(&mut row)));
                // Continuation rows keep the gutter's and the sign's columns.
                row.push(RtSpan::raw(" ".repeat(gutter_columns + 1)));
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
