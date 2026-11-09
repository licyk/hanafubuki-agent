// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the line selection of the log-select strategy of `kernel/src/crush.ts` of
// acp-kernel 0.0.189 at commit `4be21b08e`, itself ported from headroom's
// LogCompressor (headroomlabs-ai/headroom, Apache-2.0). Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the kernel's `Map` is an insertion-ordered
// table, so the chosen lines keep the kernel's order.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use super::lines::{is_go_call_frame, is_go_file_frame, is_rust_backtrace_frame, Level, LogLine};

const LOG_MAX_TOTAL_LINES: usize = 100;
// More distinct errors than this cannot all be kept, and a lossy output
// that drops one is refused: such a log passes through unchanged.
const LOG_MAX_ERRORS: usize = 20;
const LOG_ERROR_CONTEXT: usize = 3;
const LOG_MAX_WARNINGS: usize = 5;
const LOG_MAX_STACK_TRACES: usize = 3;
const LOG_STACK_MAX_LINES: usize = 20;
const LOG_TRACE_HEAD_FRAMES: usize = 3;
const LOG_TRACE_APP_FRAMES: usize = 5;

const RUNTIME_FRAME_PREFIXES: [&str; 9] = [
    "at java.",
    "at jdk.",
    "at sun.",
    "at javax.",
    "at scala.",
    "at System.",
    "at Microsoft.",
    "runtime.",
    "created by runtime.",
];
const RUNTIME_FRAME_MARKERS: [&str; 14] = [
    "site-packages/",
    "/usr/lib/python",
    "lib/python3.",
    "node:internal/",
    "node_modules/",
    "(internal/",
    "core::",
    "std::",
    "alloc::",
    "rust_begin_unwind",
    "__rust_",
    "/rustc/",
    "/usr/local/go/src/",
    "/libexec/src/runtime/",
];

/// JavaScript's `Map` from line index to line: setting a present key keeps
/// its place, deleting and setting it again moves it to the end.
#[derive(Default)]
struct Selected<'a> {
    slots: Vec<Option<LogLine<'a>>>,
    by_index: HashMap<usize, usize>,
}

impl<'a> Selected<'a> {
    fn set(&mut self, line: LogLine<'a>) {
        match self.by_index.get(&line.index) {
            Some(slot) => self.slots[*slot] = Some(line),
            None => {
                self.by_index.insert(line.index, self.slots.len());
                self.slots.push(Some(line));
            }
        }
    }

    fn delete(&mut self, index: usize) {
        if let Some(slot) = self.by_index.remove(&index) {
            self.slots[slot] = None;
        }
    }

    fn has(&self, index: usize) -> bool {
        self.by_index.contains_key(&index)
    }

    fn keys(&self) -> Vec<usize> {
        self.slots.iter().flatten().map(|line| line.index).collect()
    }

    fn into_values(self) -> Vec<LogLine<'a>> {
        self.slots.into_iter().flatten().collect()
    }
}

/// Errors and failures (first, last and the best scored, within budget),
/// the first distinct warnings, the first three traces (long ones
/// collapsed), the summaries and the context around all of them; over 100
/// lines, the best scored in log order.
pub(super) fn select_log_lines<'a>(all: &[LogLine<'a>]) -> Vec<LogLine<'a>> {
    let by_level = |level: Level| -> Vec<&LogLine<'a>> {
        all.iter().filter(|line| line.level == level).collect()
    };
    let mut stacks: Vec<Vec<&LogLine<'a>>> = Vec::new();
    let mut current = Vec::new();
    for line in all {
        if line.is_stack {
            current.push(line);
        } else if !current.is_empty() {
            stacks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        stacks.push(current);
    }

    let mut selected = Selected::default();
    for level in [Level::Error, Level::Fail] {
        for line in select_with_first_last(&by_level(level), LOG_MAX_ERRORS) {
            selected.set(line.clone());
        }
    }
    let mut seen_warnings = HashSet::new();
    by_level(Level::Warn)
        .into_iter()
        .filter(|line| seen_warnings.insert(normalize_for_dedupe(&line.content)))
        .take(LOG_MAX_WARNINGS)
        .for_each(|line| selected.set(line.clone()));
    for stack in stacks.iter().take(LOG_MAX_STACK_TRACES) {
        if stack.len() > LOG_STACK_MAX_LINES {
            let (kept, dropped) = collapse_trace_frames(stack);
            for index in dropped {
                selected.delete(index);
            }
            kept.into_iter()
                .take(LOG_STACK_MAX_LINES)
                .for_each(|line| selected.set(line));
        } else {
            stack.iter().for_each(|line| selected.set((*line).clone()));
        }
    }
    all.iter()
        .filter(|line| line.is_summary)
        .for_each(|line| selected.set(line.clone()));

    for index in selected.keys() {
        let low = index.saturating_sub(LOG_ERROR_CONTEXT);
        let high = (index + LOG_ERROR_CONTEXT + 1).min(all.len());
        for neighbour in &all[low..high] {
            if neighbour.index != index && !selected.has(neighbour.index) {
                selected.set(neighbour.clone());
            }
        }
    }

    let mut ordered = selected.into_values();
    if ordered.len() > LOG_MAX_TOTAL_LINES {
        ordered.sort_by(by_score);
        ordered.truncate(LOG_MAX_TOTAL_LINES);
        ordered.sort_by_key(|line| line.index);
    }
    ordered
}

fn by_score(a: &LogLine<'_>, b: &LogLine<'_>) -> Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(Ordering::Equal)
        .then(a.index.cmp(&b.index))
}

/// All of them when they fit, else the first, the last and the best
/// scored.
fn select_with_first_last<'a, 'b>(lines: &[&'b LogLine<'a>], max: usize) -> Vec<&'b LogLine<'a>> {
    if lines.len() <= max {
        return lines.to_vec();
    }
    let mut out: Vec<&LogLine<'a>> = Vec::new();
    let mut seen = HashSet::new();
    let ends = [lines.first(), lines.last()].into_iter().flatten();
    for line in ends {
        if seen.insert(line.index) {
            out.push(line);
        }
    }
    let mut by_score_order = lines.to_vec();
    by_score_order.sort_by(|a, b| by_score(a, b));
    for line in by_score_order {
        if out.len() >= max {
            break;
        }
        if seen.insert(line.index) {
            out.push(line);
        }
    }
    out
}

static DIGITS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new("[0-9]+").ok());
static ADDRESS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new("0x[0-9a-fA-F]+").ok());
static PATH: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new("/[A-Za-z0-9_/]+/").ok());

/// A warning's identity: the message before the first `:` or `=` as it is,
/// the variable parts after it (numbers, addresses, paths) normalised.
fn normalize_for_dedupe(content: &str) -> String {
    let split = content.find([':', '=']).unwrap_or(content.len());
    let mut suffix = content[split..].to_owned();
    for (pattern, replacement) in [(&DIGITS, "N"), (&ADDRESS, "ADDR"), (&PATH, "/PATH/")] {
        if let Some(pattern) = pattern.as_ref() {
            suffix = pattern.replace_all(&suffix, replacement).into_owned();
        }
    }
    format!("{}{suffix}", &content[..split])
}

fn is_frame_line(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("at ")
        || (trimmed.starts_with("File \"") && trimmed.contains("\", line "))
        || is_rust_backtrace_frame(content)
        || is_go_file_frame(content)
        || is_go_call_frame(content)
}

fn is_chain_head_line(content: &str) -> bool {
    let trimmed = content.trim_start();
    [
        "Caused by:",
        "Suppressed:",
        "... ",
        "--->",
        "--- End of",
        "During handling",
        "The above exception",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

fn is_runtime_frame(content: &str) -> bool {
    let trimmed = content.trim_start();
    RUNTIME_FRAME_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
        || RUNTIME_FRAME_MARKERS
            .iter()
            .any(|marker| content.contains(marker))
}

/// A long trace as its first frames, its application frames and a marker
/// for each run of dropped ones; the dropped line indexes.
fn collapse_trace_frames<'a>(stack: &[&LogLine<'a>]) -> (Vec<LogLine<'a>>, Vec<usize>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    let mut frames_seen = 0;
    let mut app_kept = 0;
    let mut run: Option<(usize, usize)> = None;
    let mut previous_dropped = false;
    let flush = |run: &mut Option<(usize, usize)>, kept: &mut Vec<LogLine<'a>>| {
        if let Some((start, length)) = run.take() {
            kept.push(LogLine {
                index: start,
                content: Cow::Owned(format!("      [... {length} frames collapsed]")),
                level: Level::Unknown,
                is_stack: true,
                is_summary: false,
                score: 0.8,
            });
        }
    };
    for line in stack {
        let content = line.content.as_ref();
        if is_frame_line(content) && !is_chain_head_line(content) {
            frames_seen += 1;
            let runtime = is_runtime_frame(content);
            if frames_seen <= LOG_TRACE_HEAD_FRAMES || (!runtime && app_kept < LOG_TRACE_APP_FRAMES)
            {
                if !runtime {
                    app_kept += 1;
                }
                flush(&mut run, &mut kept);
                kept.push((*line).clone());
                previous_dropped = false;
            } else {
                let (_, length) = run.get_or_insert((line.index, 0));
                *length += 1;
                dropped.push(line.index);
                previous_dropped = true;
            }
        } else if previous_dropped
            && (content.starts_with(' ') || content.starts_with('\t'))
            && !is_chain_head_line(content)
        {
            // A dropped frame's continuation line goes with it.
            if let Some((_, length)) = run.as_mut() {
                *length += 1;
            }
            dropped.push(line.index);
        } else {
            flush(&mut run, &mut kept);
            kept.push((*line).clone());
            previous_dropped = false;
        }
    }
    flush(&mut run, &mut kept);
    (kept, dropped)
}
