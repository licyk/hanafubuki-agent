// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A patch as the transcript shows it, Codex's `diff_render`: a header with
//! the files and their line counts, then each file's rows, each behind its
//! line number and its sign over a tint by what it does (`style`), wrapped
//! by display width with the gutter kept (`wrap`).
//!
//! Codex numbers the rows from the unified diff of the changed file. Here
//! the reading of the patch (`AgentPatchPreview`, from the parser that
//! applies it) has the rows, an added file is numbered from 1, and an
//! updated file's rows from where each chunk landed, which the applier
//! reports beside its result ([`AgentPatchFile::hunk_starts`]). A patch that
//! did not apply, or one recorded before that, shows its updates unnumbered.
//! A removed row has its number in the old file, the others in the new one;
//! a `⋮` row separates chunks.
//!
//! A file whose language its name tells is highlighted with the syntax
//! theme, a chunk at a time (an added file whole), so a construct spanning
//! lines keeps its colours; a file over the highlighter's limits is not.
//! `git_diff` is the `/diff` overlay's unified diff.

mod git_diff;
mod style;
mod wrap;

pub(crate) use git_diff::git_diff_lines;
pub(crate) use style::{current_diff_render_style_context, DiffLineType, DiffRenderStyleContext};
#[cfg(test)]
pub(crate) use style::{with_diff_style_context, DiffColorLevel, DiffTheme};
pub(crate) use wrap::{line_number_width, push_wrapped_diff_line};

use hanafubuki_agent::{
    AgentPatchFile, AgentPatchLineKind, AgentPatchOperation, AgentPatchPreview,
};
use std::path::Path;

use ratatui::style::Stylize;
use ratatui::text::{Line as RtLine, Span as RtSpan};

use crate::render::highlight::{exceeds_highlight_limits, highlight_code_to_styled_spans};
use crate::render::line_utils::line_to_static;
use crate::text_formatting::strip_terminal_controls;
use crate::wrapping::{word_wrap_line, RtOptions};
use style::style_gutter_for;

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

/// Each file's diff at `width` columns in this terminal's colours, under
/// its row when the patch has several files.
pub(crate) fn patch_diff_lines(preview: &AgentPatchPreview, width: usize) -> Vec<RtLine<'static>> {
    patch_diff_lines_with_style(preview, width, current_diff_render_style_context())
}

/// [`patch_diff_lines`] in the colours of `style`.
pub(crate) fn patch_diff_lines_with_style(
    preview: &AgentPatchPreview,
    width: usize,
    style: DiffRenderStyleContext,
) -> Vec<RtLine<'static>> {
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
            // A long path wraps under itself rather than past the width.
            let row = file_row(file);
            out.extend(
                word_wrap_line(
                    &row,
                    RtOptions::new(width.max(1)).subsequent_indent("  ".into()),
                )
                .iter()
                .map(line_to_static),
            );
        }
        render_file(file, &mut out, width, style);
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

fn render_file(
    file: &AgentPatchFile,
    out: &mut Vec<RtLine<'static>>,
    width: usize,
    style: DiffRenderStyleContext,
) {
    // A renamed file's content is the destination's, highlighted as such.
    let path = file.moved_to.as_deref().unwrap_or(&file.path);
    let texts: Vec<String> = file
        .lines
        .iter()
        .map(|line| strip_terminal_controls(&line.text))
        .collect();
    let numbers = line_numbers(file);
    let gutter = numbers
        .iter()
        .flatten()
        .max()
        .map(|max| line_number_width(*max));
    let highlighted = highlight_chunks(path, file, &texts);
    let mut seen_content = false;
    for (index, line) in file.lines.iter().enumerate() {
        let kind = match line.kind {
            AgentPatchLineKind::Added => DiffLineType::Insert,
            AgentPatchLineKind::Removed => DiffLineType::Delete,
            AgentPatchLineKind::Context => DiffLineType::Context,
            // The first chunk needs no separator from what is above it.
            AgentPatchLineKind::Hunk if !seen_content => continue,
            AgentPatchLineKind::Hunk => {
                out.push(between_rows(gutter, "⋮".dim(), style));
                continue;
            }
            AgentPatchLineKind::Marker => {
                seen_content = true;
                out.push(between_rows(gutter, texts[index].clone().dim(), style));
                continue;
            }
        };
        seen_content = true;
        out.extend(push_wrapped_diff_line(
            kind,
            &texts[index],
            highlighted[index].as_deref(),
            numbers[index].zip(gutter),
            width,
            style,
        ));
    }
    if file.omitted_lines > 0 {
        out.push(between_rows(
            gutter,
            format!("… +{} lines", file.omitted_lines).dim(),
            style,
        ));
    }
}

/// A row between a file's rows (a chunk's `⋮`, a marker), under the line
/// numbers' gutter when the file has one.
fn between_rows(
    gutter: Option<usize>,
    text: RtSpan<'static>,
    style: DiffRenderStyleContext,
) -> RtLine<'static> {
    let mut spans = Vec::with_capacity(2);
    if let Some(columns) = gutter {
        spans.push(RtSpan::styled(
            format!("{:columns$} ", ""),
            style_gutter_for(DiffLineType::Context, style.theme, style.color_level),
        ));
    }
    spans.push(text);
    RtLine::from(spans)
}

/// Each row's line number, when known: an added file's from 1; an updated
/// file's from each chunk's starts, a removed row by the old file and the
/// others by the new one.
fn line_numbers(file: &AgentPatchFile) -> Vec<Option<u64>> {
    let mut numbers = Vec::with_capacity(file.lines.len());
    match file.operation {
        AgentPatchOperation::Add => {
            let mut next = 0;
            for line in &file.lines {
                numbers.push((line.kind == AgentPatchLineKind::Added).then(|| {
                    next += 1;
                    next
                }));
            }
        }
        AgentPatchOperation::Update if !file.hunk_starts.is_empty() => {
            let mut starts = file.hunk_starts.iter();
            let mut position: Option<(u64, u64)> = None;
            for line in &file.lines {
                let number = match (line.kind, position.as_mut()) {
                    (AgentPatchLineKind::Hunk, _) => {
                        position = starts.next().map(|start| (start.old_line, start.new_line));
                        None
                    }
                    (AgentPatchLineKind::Removed, Some((old, _))) => {
                        *old += 1;
                        Some(*old - 1)
                    }
                    (AgentPatchLineKind::Added, Some((_, new))) => {
                        *new += 1;
                        Some(*new - 1)
                    }
                    (AgentPatchLineKind::Context, Some((old, new))) => {
                        *old += 1;
                        *new += 1;
                        Some(*new - 1)
                    }
                    _ => None,
                };
                numbers.push(number);
            }
        }
        _ => numbers.resize(file.lines.len(), None),
    }
    numbers
}

/// Each row's highlighted spans: every chunk read as one piece of code (an
/// added file is one chunk), so its parser state runs across its rows but
/// not from one chunk into the next, which may be far away. None when the
/// file's name tells no language or the file is over the limits.
fn highlight_chunks(
    path: &str,
    file: &AgentPatchFile,
    texts: &[String],
) -> Vec<Option<Vec<RtSpan<'static>>>> {
    let mut highlighted = vec![None; file.lines.len()];
    let code = |index: &usize| is_code(file.lines[*index].kind);
    let total_bytes = (0..texts.len())
        .filter(code)
        .map(|index| texts[index].len() + 1)
        .sum();
    let total_lines = (0..texts.len()).filter(code).count();
    let Some(lang) = language(path) else {
        return highlighted;
    };
    if exceeds_highlight_limits(total_bytes, total_lines) {
        return highlighted;
    }
    let mut chunk: Vec<usize> = Vec::new();
    for index in 0..=texts.len() {
        let ends = index == texts.len() || file.lines[index].kind == AgentPatchLineKind::Hunk;
        if !ends {
            if code(&index) {
                chunk.push(index);
            }
            continue;
        }
        if chunk.is_empty() {
            continue;
        }
        let source: String = chunk
            .iter()
            .flat_map(|index| [texts[*index].as_str(), "\n"])
            .collect();
        if let Some(lines) = highlight_code_to_styled_spans(&source, lang) {
            if lines.len() == chunk.len() {
                for (index, spans) in chunk.iter().zip(lines) {
                    highlighted[*index] = Some(spans);
                }
            }
        }
        chunk.clear();
    }
    highlighted
}

fn is_code(kind: AgentPatchLineKind) -> bool {
    matches!(
        kind,
        AgentPatchLineKind::Added | AgentPatchLineKind::Removed | AgentPatchLineKind::Context
    )
}

/// The language a file's name tells: its extension, else its name
/// (`Makefile`, `Dockerfile`).
fn language(path: &str) -> Option<&str> {
    let path = Path::new(path);
    path.extension()
        .or_else(|| path.file_name())
        .and_then(|lang| lang.to_str())
}

/// `lines` of the file at `path` highlighted as one piece of code, when
/// its name tells its language and it is not too large.
pub(crate) fn highlight_lines(path: &str, lines: &[String]) -> Option<Vec<Vec<RtSpan<'static>>>> {
    let total_bytes = lines.iter().map(|line| line.len() + 1).sum();
    if lines.is_empty() || exceeds_highlight_limits(total_bytes, lines.len()) {
        return None;
    }
    // Each line with its own ending, so a last empty line is one.
    let code: String = lines
        .iter()
        .flat_map(|line| [line.as_str(), "\n"])
        .collect();
    let highlighted = highlight_code_to_styled_spans(&code, language(path)?)?;
    (highlighted.len() == lines.len()).then_some(highlighted)
}

#[cfg(test)]
mod tests;
