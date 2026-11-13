// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A command and what it printed (`exec_command`, `write_stdin`), after
//! Codex's exec cell. Collapsed: `Ran` and the command, the output's last
//! lines (or `(no output)`), and the exit when it failed. Expanded: `$` and
//! the command, the whole output as the result returned it, and how it
//! ended. The output reaches the interface as plain text (the execution
//! plugin renders the terminal's), so it has no colours to keep.
//!
//! While the command runs, and until its display arrives, the same rows show
//! what its process printed so far (`streamed`).
//!
//! A command the person typed (`!command`) says `You ran`, as Codex's; one
//! that failed adds where it ran, since the sandbox may be why.

use std::time::Duration;

use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use super::live_output::LiveOutput;
use super::{clipped, text_lines, wrapped, Extent, ToolCall, View, EXEC_COMMAND, PREVIEW_LINES};
use crate::history_cell::ActivityDisclosure;
use crate::render::highlight::highlight_code_to_lines;
use crate::status_indicator_widget::fmt_elapsed_tenths;
use crate::text_formatting::strip_terminal_controls;

/// The columns of a command line in the header, a subject's bound.
const TITLE_COLUMNS: usize = 200;
/// Under a failed `!command`: the sandbox may have refused what it did.
const SANDBOX_HINT: &str = "`!` commands run in the session's sandbox (/permissions)";

pub(super) struct Command<'a> {
    pub(super) command: &'a str,
    pub(super) cwd: Option<&'a str>,
    pub(super) output: &'a str,
    pub(super) exit_code: Option<i32>,
    pub(super) duration_millis: Option<u64>,
}

pub(super) fn view(call: &ToolCall, command: &Command, failed: bool, extent: Extent) -> View {
    let failed = failed || command.exit_code.is_some_and(|code| code != 0);
    // `write_stdin` typed into a session, or waited for it.
    let interaction = call.name != EXEC_COMMAND;
    let script = strip_terminal_controls(command.command);
    let script_lines = highlight_code_to_lines(&script, "bash");
    let mut title_cut = false;
    let title = if interaction {
        let verb = if script.is_empty() {
            "Waited for"
        } else {
            "Interacted with"
        };
        vec![verb.bold(), " ".into(), call.subject_or(&call.name).into()]
    } else {
        let verb = if call.by_user { "You ran" } else { "Ran" };
        let mut title = vec![verb.bold(), " ".into()];
        // A long command line is cut in the header, as a subject is.
        let (first, was_cut) = clipped(
            script_lines.first().cloned().unwrap_or_default(),
            TITLE_COLUMNS,
        );
        title_cut = was_cut;
        title.extend(first.spans);
        if script_lines.len() > 1 {
            title.push(" …".dim());
        }
        title
    };

    let output = text_lines(command.output);
    let (mut preview, cut) = last_rows(&output, extent);
    if output.is_empty() && !interaction {
        preview.push(Line::from("(no output)".dim()));
    }
    match command.exit_code {
        Some(0) => {}
        Some(code) => preview.push(Line::from(with_time(format!("exit {code}"), command).red())),
        // A command that went on in the background; a session's output
        // goes on whatever the call returned.
        None if !interaction => preview.push(ending(command)),
        None => {}
    }

    let mut details = Vec::new();
    if extent.expanded {
        if !interaction {
            details = script_rows(&script_lines, extent);
        }
        details.extend(output_rows(&output, extent));
        details.push(ending(command));
    }
    if call.by_user && failed {
        preview.push(Line::from(SANDBOX_HINT.dim()));
        if extent.expanded {
            details.push(Line::from(SANDBOX_HINT.dim()));
        }
    }

    View {
        failed,
        title,
        preview,
        details,
        hidden: hidden(output.len(), cut || title_cut || script_lines.len() > 1),
    }
}

/// `header` with what the call's process printed so far under it: the last
/// lines, as the finished command shows its output; expanded, the command
/// as it started and the lines kept.
pub(super) fn streamed(call: &ToolCall, header: View, live: &LiveOutput, extent: Extent) -> View {
    let (output, count) = live.lines();
    let (preview, cut) = last_rows(&output, extent);
    let mut details = Vec::new();
    if extent.expanded {
        if call.name == EXEC_COMMAND {
            if let Some(subject) = &call.subject {
                let script = highlight_code_to_lines(&strip_terminal_controls(subject), "bash");
                details = script_rows(&script, extent);
            }
        }
        details.extend(output_rows(&output, extent));
    }
    View {
        preview,
        details,
        hidden: hidden(count, cut),
        ..header
    }
}

/// The last of `output`'s lines, each cut to a row, and whether one was.
fn last_rows(output: &[String], extent: Extent) -> (Vec<Line<'static>>, bool) {
    let mut rows = Vec::new();
    let mut cut = false;
    for line in &output[output.len().saturating_sub(PREVIEW_LINES)..] {
        let (row, was_cut) = clipped(Line::from(line.clone()).dim(), extent.columns);
        cut |= was_cut;
        rows.push(row);
    }
    (rows, cut)
}

/// `$` and the command, wrapped.
fn script_rows(script_lines: &[Line<'static>], extent: Extent) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for (index, line) in script_lines.iter().enumerate() {
        let mut spans = vec![if index == 0 { "$ " } else { "  " }.dim()];
        spans.extend(line.spans.iter().cloned());
        rows.extend(wrapped(Line::from(spans), extent.columns, "  "));
    }
    rows
}

fn output_rows(output: &[String], extent: Extent) -> Vec<Line<'static>> {
    output
        .iter()
        .flat_map(|line| wrapped(Line::from(line.clone()), extent.columns, ""))
        .collect()
}

/// What expanding adds to `lines` lines of output, the last of them shown;
/// `more` when expanding shows more of what is shown.
fn hidden(lines: usize, more: bool) -> Option<ActivityDisclosure> {
    if lines > PREVIEW_LINES {
        Some(ActivityDisclosure::OutputLines(lines - PREVIEW_LINES))
    } else {
        more.then_some(ActivityDisclosure::Generic)
    }
}

/// `✓ 4.2s`, `✗ exit 101 · 4.2s`, or `still running` for a command that
/// went on in the background; then where it ran.
fn ending(command: &Command) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = match command.exit_code {
        Some(0) => vec!["✓".green(), with_time(String::new(), command).dim()],
        Some(code) => vec![format!("✗ {}", with_time(format!("exit {code}"), command)).red()],
        None => vec![with_time("still running".to_string(), command).dim()],
    };
    if let Some(cwd) = command.cwd {
        spans.push(format!(" · in {}", strip_terminal_controls(cwd)).dim());
    }
    Line::from(spans)
}

/// `label · 4.2s`, or ` 4.2s` after a mark without a label.
fn with_time(label: String, command: &Command) -> String {
    let Some(millis) = command.duration_millis else {
        return label;
    };
    let time = fmt_elapsed_tenths(Duration::from_millis(millis));
    if label.is_empty() {
        format!(" {time}")
    } else {
        format!("{label} · {time}")
    }
}
