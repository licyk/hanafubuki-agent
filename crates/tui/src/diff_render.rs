// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! A patch as the transcript shows it: a header with the files and their
//! line counts, then each file's lines with a sign column. Codex reads a
//! unified diff of the changed file and numbers its lines; the Agent's
//! reading of a patch (`PatchPreview`, the parser that applies it) has no
//! line numbers, so this has no gutter, and a chunk header is a `⋮` row.

use hanafubuki_agent::{
    PatchPreview, PatchPreviewFile, PatchPreviewLineKind, PatchPreviewOperation,
};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line as RtLine, Span as RtSpan};

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
    let mut seen_content = false;
    for line in &file.lines {
        let text = strip_terminal_controls(&line.text);
        match line.kind {
            PatchPreviewLineKind::Added => {
                out.extend(push_wrapped_diff_line('+', &text, style_add(), width))
            }
            PatchPreviewLineKind::Removed => {
                out.extend(push_wrapped_diff_line('-', &text, style_del(), width))
            }
            PatchPreviewLineKind::Context => {
                out.extend(push_wrapped_diff_line(' ', &text, style_context(), width))
            }
            // The first chunk needs no separator from what is above it.
            PatchPreviewLineKind::Hunk if !seen_content => {}
            PatchPreviewLineKind::Hunk => out.push(RtLine::from("⋮".dim())),
            PatchPreviewLineKind::Marker => out.push(RtLine::from(text).dim()),
        }
        seen_content |= !matches!(line.kind, PatchPreviewLineKind::Hunk);
    }
    if file.omitted_lines > 0 {
        out.push(RtLine::from(format!("… +{} lines", file.omitted_lines)).dim());
    }
}

fn push_wrapped_diff_line(
    sign_char: char,
    text: &str,
    line_style: Style,
    width: usize,
) -> Vec<RtLine<'static>> {
    let mut remaining_text: &str = text;
    let mut first = true;
    let mut lines: Vec<RtLine<'static>> = Vec::new();
    loop {
        // Split at a character boundary so this row's chunk fits the
        // columns left after the sign.
        let available_content_cols = width.saturating_sub(1).max(1);
        let split_at_byte_index = remaining_text
            .char_indices()
            .nth(available_content_cols)
            .map(|(i, _)| i)
            .unwrap_or_else(|| remaining_text.len());
        let (chunk, rest) = remaining_text.split_at(split_at_byte_index);
        remaining_text = rest;
        let content = if first {
            format!("{sign_char}{chunk}")
        } else {
            // Continuation lines keep a space for the sign column.
            format!(" {chunk}")
        };
        lines.push(RtLine::from(RtSpan::styled(content, line_style)));
        first = false;
        if remaining_text.is_empty() {
            break;
        }
    }
    lines
}

fn style_context() -> Style {
    Style::default()
}

fn style_add() -> Style {
    Style::default().fg(Color::Green)
}

fn style_del() -> Style {
    Style::default().fg(Color::Red)
}

#[cfg(test)]
mod tests;
