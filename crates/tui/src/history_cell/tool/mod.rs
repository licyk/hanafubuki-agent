// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A tool call: a live entry while it runs (with a spinner), then what its
//! presenter's display shows, collapsed by default. Codex's exec, patch and
//! MCP cells, for every tool the Agent has: a frame (bullet, verb, subject)
//! with a body by display, each in its own module, its rows under `└`.
//! Expanded (the control, Ctrl-T's transcript) shows the display whole.
//!
//! A call keeps its shape while it changes: from its start until what it
//! shows arrives, a dim row stands under its header (`(running…)` for a
//! command, `…` otherwise) where its first rows will be, and a command's
//! clock goes after its header only where its widest fits, so the header
//! wraps the same as the clock counts and as it will once finished.
//!
//! Consecutive read, list and search calls share one entry, Codex's
//! exploration cell (`exploration`): its activity ids are its calls'.

mod command;
mod exploration;
mod file_read;
mod listing;
mod live_output;
mod mutation;
mod patch;
mod search;
mod text;
mod web;

use std::time::Instant;

use hanafubuki_agent::{AgentToolCallDisplay, AgentToolDisplay};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use live_output::LiveOutput;

use super::{plain_lines, ActivityDisclosure, HistoryCell};
use crate::render::line_utils::{line_to_static, prefix_lines};
use crate::shimmer::spinner;
use crate::status_indicator_widget::fmt_elapsed_tenths;
use crate::text_formatting::strip_terminal_controls;
use crate::wrapping::{word_wrap_line, RtOptions};

const EXEC_COMMAND: &str = "exec_command";
const READ_FILE: &str = "read_file";
const LIST_DIRECTORY: &str = "list_directory";
const SEARCH_TEXT: &str = "search_text";
/// The tool whose output is folded to its receipt's first line.
const COMPRESS: &str = "compress";
/// The rows of output a collapsed entry shows.
const PREVIEW_LINES: usize = 3;
/// The spinner's period while a call runs.
const SPINNER_TICK_MILLIS: u128 = 600;
/// A running command's clock's period, in tenths of a second.
const CLOCK_TICK_MILLIS: u128 = 100;
/// The columns kept for a running command's clock at the end of its
/// header: its widest, ` · 99h 59m 59s`.
const CLOCK_COLUMNS: usize = 14;

/// Whether consecutive calls of the tool share one entry. Decided by name:
/// a call joins its entry when it starts, before any display.
pub(crate) fn explores(name: &str) -> bool {
    matches!(name, READ_FILE | LIST_DIRECTORY | SEARCH_TEXT)
}

/// One entry: a call, or a run of exploring calls.
#[derive(Debug)]
pub(crate) struct ToolCallCell {
    calls: Vec<ToolCall>,
}

#[derive(Debug)]
struct ToolCall {
    call_id: String,
    name: String,
    subject: Option<String>,
    start_time: Instant,
    outcome: Option<Outcome>,
    /// What its process printed while it ran, until its display arrives.
    live: Option<LiveOutput>,
}

#[derive(Debug)]
struct Outcome {
    is_error: bool,
    /// What it shows; `None` until its display arrives, and when there is
    /// none.
    display: Option<AgentToolDisplay>,
    /// Its display arrived, or it will have none.
    shown: bool,
}

pub(crate) fn new_tool_call(
    call_id: String,
    name: String,
    subject: Option<String>,
) -> ToolCallCell {
    ToolCallCell {
        calls: vec![ToolCall {
            call_id,
            name,
            subject,
            start_time: Instant::now(),
            outcome: None,
            live: None,
        }],
    }
}

impl ToolCallCell {
    /// The calls of `other` after this entry's.
    pub(crate) fn join(&mut self, other: ToolCallCell) {
        self.calls.extend(other.calls);
    }

    /// Output the running call `call_id`'s process printed (see
    /// [`LiveOutput::push`]).
    pub(crate) fn push_output(&mut self, call_id: &str, text: &str, truncated: bool) {
        if let Some(call) = self.call_mut(call_id) {
            call.live
                .get_or_insert_with(LiveOutput::default)
                .push(text, truncated);
        }
    }

    /// The turn ended before the calls still running finished: they end
    /// as they stood, failed, so no spinner or clock runs on.
    pub(crate) fn end_running(&mut self) {
        for call in &mut self.calls {
            call.outcome.get_or_insert(Outcome {
                is_error: true,
                display: None,
                shown: true,
            });
        }
    }

    pub(crate) fn complete(&mut self, call_id: &str, is_error: bool) {
        if let Some(call) = self.call_mut(call_id) {
            call.outcome = Some(Outcome {
                is_error,
                display: None,
                shown: false,
            });
        }
    }

    /// What the call's presenter shows: its subject, when it gives one,
    /// and the finished call's display, `None` when it has none.
    pub(crate) fn show(&mut self, call_id: &str, shown: AgentToolCallDisplay) {
        let Some(call) = self.call_mut(call_id) else {
            return;
        };
        if shown.subject.is_some() {
            call.subject = shown.subject;
        }
        if let Some(outcome) = &mut call.outcome {
            if shown.display.is_some() {
                call.live = None;
            }
            outcome.display = shown.display;
            outcome.shown = true;
        }
    }

    /// The call `call_id` started `ago`.
    #[cfg(test)]
    pub(crate) fn started_ago(&mut self, call_id: &str, ago: std::time::Duration) {
        if let Some(call) = self.call_mut(call_id) {
            call.start_time = Instant::now() - ago;
        }
    }

    /// The call `call_id` finished, showing `display`.
    #[cfg(test)]
    pub(crate) fn finish(&mut self, call_id: &str, display: AgentToolDisplay) {
        self.complete(call_id, false);
        self.show(
            call_id,
            AgentToolCallDisplay {
                subject: None,
                display: Some(display),
            },
        );
    }

    fn call_mut(&mut self, call_id: &str) -> Option<&mut ToolCall> {
        self.calls.iter_mut().find(|call| call.call_id == call_id)
    }

    fn lines(&self, width: u16, expanded: bool) -> Vec<Line<'static>> {
        match &self.calls[..] {
            [call] => {
                let view = call.view(Extent::new(width, expanded));
                let body = if expanded { view.details } else { view.preview };
                let bullet = call.bullet(view.failed);
                frame(bullet, view.title, call.clock(), body, width)
            }
            calls => exploration::lines(calls, width, expanded),
        }
    }
}

impl HistoryCell for ToolCallCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines(width, false)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines(width, true)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.lines(u16::MAX, true))
    }

    fn activity_ids(&self) -> Vec<String> {
        self.calls
            .iter()
            .map(|call| format!("tool:{}", call.call_id))
            .collect()
    }

    fn activity_disclosure(&self, width: u16) -> Option<ActivityDisclosure> {
        match &self.calls[..] {
            [call] => call.view(Extent::new(width, false)).hidden,
            // A run's details always show more than its rows: each call's
            // own view.
            _ => Some(ActivityDisclosure::Generic),
        }
    }

    /// The spinner blinks while a call runs, and a command's clock counts.
    fn transcript_animation_tick(&self) -> Option<u64> {
        let call = self.calls.iter().find(|call| call.outcome.is_none())?;
        let period = if call.name == EXEC_COMMAND {
            CLOCK_TICK_MILLIS
        } else {
            SPINNER_TICK_MILLIS
        };
        let ticks = call.start_time.elapsed().as_millis() / period;
        Some(u64::try_from(ticks).unwrap_or(u64::MAX))
    }
}

/// The columns of the rows under a header, after their `└` prefix, and
/// whether the expanded rows are laid out: a collapsed entry does without
/// them (a read's highlighting, an output's wrapping).
#[derive(Clone, Copy)]
struct Extent {
    columns: usize,
    expanded: bool,
}

impl Extent {
    fn new(width: u16, expanded: bool) -> Self {
        Self {
            columns: usize::from(width).saturating_sub(4).max(1),
            expanded,
        }
    }

    /// The same, two columns narrower, for rows nested under a row.
    fn nested(self) -> Self {
        Self {
            columns: self.columns.saturating_sub(2).max(1),
            ..self
        }
    }
}

/// What one call shows: its header after the bullet, and the rows under
/// it, collapsed and, when its extent is, expanded.
struct View {
    failed: bool,
    title: Vec<Span<'static>>,
    preview: Vec<Line<'static>>,
    details: Vec<Line<'static>>,
    /// What expanding adds; `None` when the collapsed rows show it all.
    hidden: Option<ActivityDisclosure>,
}

impl View {
    /// A header alone.
    fn title(title: Vec<Span<'static>>, failed: bool) -> Self {
        Self {
            failed,
            title,
            preview: Vec::new(),
            details: Vec::new(),
            hidden: None,
        }
    }
}

impl ToolCall {
    fn view(&self, extent: Extent) -> View {
        let failed = self
            .outcome
            .as_ref()
            .is_some_and(|outcome| outcome.is_error);
        if let Some(display) = self
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.display.as_ref())
        {
            let mut view = self.display_view(display, failed, extent);
            view.failed |= failed;
            return view;
        }
        // Until its display arrives, and when none does, a call keeps what
        // its process printed under its header.
        let header = match self.outcome {
            None => text::running(self),
            Some(_) => text::called(self, failed),
        };
        let mut view = match &self.live {
            Some(live) => command::streamed(self, header, live, extent),
            None => header,
        };
        if self.outcome.as_ref().is_none_or(|outcome| !outcome.shown) {
            for rows in [&mut view.preview, &mut view.details] {
                if rows.is_empty() {
                    rows.push(self.placeholder());
                }
            }
        }
        view
    }

    /// The row a call shows under its header until its first rows come.
    fn placeholder(&self) -> Line<'static> {
        let running_command = self.outcome.is_none() && self.name == EXEC_COMMAND;
        Line::from(
            if running_command {
                "(running…)"
            } else {
                "…"
            }
            .dim(),
        )
    }

    /// The time a running command has run so far, ` · 3.4s`.
    fn clock(&self) -> Option<Span<'static>> {
        (self.outcome.is_none() && self.name == EXEC_COMMAND)
            .then(|| format!(" · {}", fmt_elapsed_tenths(self.start_time.elapsed())).dim())
    }

    fn display_view(&self, display: &AgentToolDisplay, failed: bool, extent: Extent) -> View {
        match display {
            AgentToolDisplay::Command {
                command,
                cwd,
                output,
                exit_code,
                duration_millis,
                ..
            } => command::view(
                self,
                &command::Command {
                    command,
                    cwd: cwd.as_deref(),
                    output,
                    exit_code: *exit_code,
                    duration_millis: *duration_millis,
                },
                failed,
                extent,
            ),
            AgentToolDisplay::Patch { preview } => patch::view(preview, failed, extent),
            AgentToolDisplay::FileRead {
                path,
                start_line,
                text,
                truncated,
            } => file_read::view(self, path, *start_line, text, *truncated, extent),
            AgentToolDisplay::Listing {
                path,
                entries,
                truncated,
            } => listing::view(self, path, entries, *truncated, extent),
            AgentToolDisplay::Search {
                query,
                path,
                matches,
                truncated,
            } => search::view(self, query, path.as_deref(), matches, *truncated, extent),
            AgentToolDisplay::WebSearch { query, results } => {
                web::search_view(query, results, extent)
            }
            AgentToolDisplay::WebPage {
                url, title, text, ..
            } => web::page_view(url, title.as_deref(), text, extent),
            AgentToolDisplay::Mutation { kind, paths } => mutation::view(self, *kind, paths),
            AgentToolDisplay::Text { text } => text::view(self, failed, text_lines(text), extent),
            AgentToolDisplay::Fields { rows } => text::view(
                self,
                failed,
                rows.iter()
                    .map(|(key, value)| strip_terminal_controls(&format!("{key}: {value}")))
                    .collect(),
                extent,
            ),
        }
    }

    fn bullet(&self, failed: bool) -> Span<'static> {
        match &self.outcome {
            None => spinner(Some(self.start_time), true),
            Some(_) if failed => "•".red().bold(),
            Some(_) => "•".green().bold(),
        }
    }

    /// The subject the call started or was shown with, else `fallback`.
    fn subject_or(&self, fallback: &str) -> String {
        strip_terminal_controls(self.subject.as_deref().unwrap_or(fallback))
    }
}

/// The header (`bullet` and `title`, wrapped under itself, then `clock`),
/// then `body` under `└`. The clock follows the header's last row when its
/// widest fits there and is left out otherwise: it never adds a row, and
/// whether it shows never depends on the time it reads.
fn frame(
    bullet: Span<'static>,
    title: Vec<Span<'static>>,
    clock: Option<Span<'static>>,
    body: Vec<Line<'static>>,
    width: u16,
) -> Vec<Line<'static>> {
    let columns = usize::from(width).max(1);
    let mut spans = vec![bullet, " ".into()];
    spans.extend(title);
    let mut lines = wrapped(Line::from(spans), columns, "    ");
    if let Some(last) = lines
        .last_mut()
        .filter(|last| last.width() + CLOCK_COLUMNS <= columns)
    {
        last.spans.extend(clock);
    }
    lines.extend(prefix_lines(body, "  └ ".dim(), "    ".into()));
    lines
}

/// `line` wrapped at `columns`, its later rows behind `indent`.
fn wrapped(
    line: Line<'static>,
    columns: usize,
    indent: impl Into<Line<'static>>,
) -> Vec<Line<'static>> {
    word_wrap_line(
        &line,
        RtOptions::new(columns).subsequent_indent(indent.into()),
    )
    .iter()
    .map(line_to_static)
    .collect()
}

/// `line` cut to `columns` with an ellipsis, and whether it was cut.
fn clipped(line: Line<'static>, columns: usize) -> (Line<'static>, bool) {
    if line.width() <= columns {
        return (line, false);
    }
    let mut room = columns.saturating_sub(1);
    let mut spans = Vec::new();
    for span in line.spans {
        if room == 0 {
            break;
        }
        let mut content = String::new();
        for ch in span.content.chars() {
            let ch_width = ch.to_string().width();
            if ch_width > room {
                room = 0;
                break;
            }
            room -= ch_width;
            content.push(ch);
        }
        spans.push(Span::styled(content, span.style));
    }
    spans.push("…".dim());
    (Line::from(spans).style(line.style), true)
}

/// The lines of an output, without terminal controls and trailing blank
/// lines.
fn text_lines(text: &str) -> Vec<String> {
    strip_terminal_controls(text)
        .trim_end()
        .lines()
        .map(expand_tabs)
        .collect()
}

/// `line` with each tab replaced by spaces up to the next stop, as the
/// transcript lays a tab out, so its width is measured as it shows.
fn expand_tabs(line: &str) -> String {
    const TAB_STOP: usize = 4;
    let mut expanded = String::with_capacity(line.len());
    let mut column = 0;
    for ch in line.chars() {
        if ch == '\t' {
            let spaces = TAB_STOP - column % TAB_STOP;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(ch);
            column += ch.to_string().width();
        }
    }
    expanded
}

/// `1 match`, `7 matches`.
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

#[cfg(test)]
mod tests;
