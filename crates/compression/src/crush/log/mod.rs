// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the log-select strategy of `kernel/src/crush.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e`, itself ported from headroom's LogCompressor
// (headroomlabs-ai/headroom, Apache-2.0). Copyright (c) 2026 ranxianglei. MIT
// License with an additional attribution term (NOTICE).

mod lines;
mod select;

use std::collections::HashSet;

use crate::text::utf16_len;
use lines::{
    classify_level, is_log_summary_line, parse_log_lines, trace_flavor_for, Level, LogLine,
};

const LOG_MIN_LINES: usize = 50;
const LOG_CLASSIFIED_GATE: usize = 5;
const LOG_SUMMARY_GATE: usize = 3;

const LOG_FORMATS: [(&str, &[&str]); 5] = [
    (
        "pytest",
        &[
            "=== FAILURES",
            "=== ERRORS",
            "=== test session",
            "=== short test summary",
            "PASSED [",
            "FAILED [",
            "ERROR [",
            "SKIPPED [",
            "collected ",
        ],
    ),
    ("npm", &["npm ERR!", "npm WARN", "npm info", "npm http"]),
    (
        "cargo",
        &[
            "Compiling ",
            "Finished ",
            "Running ",
            "warning: ",
            "error[E",
        ],
    ),
    ("jest", &["PASS ", "FAIL ", "Test Suites:"]),
    ("make", &["make[", "make:", "gcc ", "g++ ", "clang "]),
];

/// Condenses a build or test log: errors and failures first, deduplicated
/// warnings, bounded stack traces (runtime frames collapsed), summary lines
/// and three lines of context around them, at most 100 lines, and a footer
/// counting what was left out. Lines are kept in the order they were
/// chosen, not the log's (the kernel's order). `None` for fewer than 50
/// lines, a text without log structure, or nothing saved.
pub fn crush_log(text: &str) -> Option<String> {
    let lines = split_lines(text);
    if lines.len() < LOG_MIN_LINES {
        return None;
    }
    let classified = lines
        .iter()
        .filter(|line| classify_level(line) != Level::Unknown)
        .count();
    let summaries = lines
        .iter()
        .filter(|line| is_log_summary_line(line))
        .count();
    let traces = lines
        .iter()
        .filter(|line| trace_flavor_for(line).is_some())
        .count();
    // Prose and structured data must never reach the selector.
    let structured = classified >= LOG_CLASSIFIED_GATE
        || traces >= 1
        || summaries >= LOG_SUMMARY_GATE
        || detect_log_format(&lines).is_some();
    if !structured {
        return None;
    }
    let parsed = parse_log_lines(&lines);
    let selected = select::select_log_lines(&parsed);
    if selected.len() >= lines.len() {
        return None;
    }
    let out = format_log_output(&selected, &parsed);
    (utf16_len(&out) < utf16_len(text)).then_some(out)
}

fn detect_log_format(lines: &[&str]) -> Option<&'static str> {
    let mut best = None;
    let mut best_score = 0;
    for (name, patterns) in LOG_FORMATS {
        let score = lines
            .iter()
            .take(100)
            .filter(|line| patterns.iter().any(|pattern| line.contains(pattern)))
            .count();
        if score > best_score {
            best_score = score;
            best = Some(name);
        }
    }
    best
}

fn format_log_output(selected: &[LogLine<'_>], all: &[LogLine<'_>]) -> String {
    let mut output: Vec<&str> = selected.iter().map(|line| line.content.as_ref()).collect();
    let omitted = all.len() - selected.len();
    let footer;
    if omitted > 0 {
        let count = |level: Level| all.iter().filter(|line| line.level == level).count();
        let parts: Vec<String> = [
            (Level::Error, "ERROR"),
            (Level::Fail, "FAIL"),
            (Level::Warn, "WARN"),
            (Level::Info, "INFO"),
        ]
        .into_iter()
        .filter_map(|(level, name)| {
            let count = count(level);
            (count > 0).then(|| format!("{count} {name}"))
        })
        .collect();
        if !parts.is_empty() {
            footer = format!("[{omitted} lines omitted: {}]", parts.join(", "));
            output.push(&footer);
        }
    }
    output.join("\n")
}

/// Whether every error and failure line of `input` is in `output`,
/// compared trimmed: a lossy log output that drops one is refused.
pub(super) fn error_lines_survive(input: &str, output: &str) -> bool {
    let kept: HashSet<&str> = split_lines(output)
        .into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    split_lines(input)
        .into_iter()
        .filter(|line| matches!(classify_level(line), Level::Error | Level::Fail))
        .map(str::trim)
        .all(|line| line.is_empty() || kept.contains(line))
}

/// `text.split(/\r?\n/)`.
pub(super) fn split_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    for line in &mut lines[..last] {
        *line = line.strip_suffix('\r').unwrap_or(line);
    }
    lines
}
