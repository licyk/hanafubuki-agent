// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the line classification of the log-select strategy of `kernel/src/crush.ts`
// of acp-kernel 0.0.189 at commit `4be21b08e`, itself ported from headroom's
// LogCompressor (headroomlabs-ai/headroom, Apache-2.0). Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

const LOG_STACK_MAX_LINES: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Error,
    Fail,
    Warn,
    Info,
    Debug,
    Trace,
    Unknown,
}

#[derive(Clone, Debug)]
pub(super) struct LogLine<'a> {
    pub(super) index: usize,
    pub(super) content: Cow<'a, str>,
    pub(super) level: Level,
    pub(super) is_stack: bool,
    pub(super) is_summary: bool,
    pub(super) score: f64,
}

// By severity, not position: a line naming both a warning and an error is
// an error, kept by the error budget.
static LEVELS: LazyLock<Vec<(Level, Regex)>> = LazyLock::new(|| {
    [
        (Level::Error, "ERROR|FATAL|CRITICAL"),
        (Level::Fail, "FAIL|FAILED"),
        (Level::Warn, "WARN|WARNING"),
        (Level::Info, "INFO"),
        (Level::Debug, "DEBUG"),
        (Level::Trace, "TRACE"),
    ]
    .into_iter()
    .filter_map(|(level, words)| {
        Some((
            level,
            Regex::new(&format!(r"(?i)(?-u:\b)({words})(?-u:\b)")).ok()?,
        ))
    })
    .collect()
});

pub(super) fn classify_level(line: &str) -> Level {
    LEVELS
        .iter()
        .find(|(_, pattern)| pattern.is_match(line))
        .map_or(Level::Unknown, |(level, _)| *level)
}

pub(super) fn is_log_summary_line(line: &str) -> bool {
    if line.starts_with("===") || line.starts_with("---") {
        return true;
    }
    let digits = leading_digits(line);
    if digits > 0 && line[digits..].starts_with(' ') {
        let rest = &line[digits + 1..];
        if ["passed", "failed", "skipped", "error", "warning"]
            .iter()
            .any(|word| rest.starts_with(word))
        {
            return true;
        }
    }
    let counted = [
        "Test ", "Tests ", "Tests:", "Test:", "Suite ", "Suites ", "Suites:", "Suite:",
    ]
    .iter()
    .any(|prefix| {
        line.strip_prefix(prefix).is_some_and(|rest| {
            rest.chars()
                .find(|character| !character.is_whitespace())
                .is_some_and(|character| character.is_ascii_digit())
        })
    });
    if counted
        || ["TOTAL", "Total", "Summary"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
    {
        return true;
    }
    ["Build", "Compile", "Test"].iter().any(|prefix| {
        line.starts_with(prefix)
            && ["succeeded", "failed", "complete"]
                .iter()
                .any(|word| line.contains(word))
    })
}

fn leading_digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

fn is_ascii_alnum(character: char) -> bool {
    character.is_ascii_alphanumeric()
}

/// `:12:34` somewhere in the text.
fn has_line_col_suffix(text: &str) -> bool {
    let bytes = text.as_bytes();
    (0..bytes.len()).any(|index| {
        bytes[index] == b':' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit) && {
            let end = index + 1 + leading_digits(&text[index + 1..]);
            bytes.get(end) == Some(&b':') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit)
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flavor {
    Python,
    JavaScript,
    Java,
    Dotnet,
    RustError,
    RustBacktrace,
    GoPanic,
}

fn is_python_file_frame(text: &str) -> bool {
    text.starts_with("File \"")
        && text.contains("\", line ")
        && text.ends_with(|character: char| character.is_ascii_digit())
}

fn is_js_at_frame(text: &str) -> bool {
    text.starts_with("at ") && text.contains('(') && text.contains(')') && has_line_col_suffix(text)
}

fn is_java_at_frame(text: &str) -> bool {
    if !text.starts_with("at ") {
        return false;
    }
    let Some(open) = text.find('(') else {
        return false;
    };
    let body = text.get(3..open).unwrap_or("");
    !body.is_empty()
        && body
            .chars()
            .all(|character| is_ascii_alnum(character) || "._$/".contains(character))
}

fn is_goroutine_header(line: &str) -> bool {
    line.strip_prefix("goroutine ").is_some_and(|rest| {
        let digits = leading_digits(rest);
        digits > 0 && rest[digits..].starts_with(" [")
    })
}

fn is_go_panic_opener(line: &str) -> bool {
    line.starts_with("panic: ") || line.starts_with("fatal error: ") || is_goroutine_header(line)
}

pub(super) fn is_go_file_frame(line: &str) -> bool {
    line.starts_with('\t') && line.contains(".go:") && line.contains(" +0x")
}

pub(super) fn is_go_call_frame(line: &str) -> bool {
    if line.starts_with("created by ") {
        return true;
    }
    if line.starts_with(' ') || line.starts_with('\t') || !line.ends_with(')') {
        return false;
    }
    let Some(open) = line.find('(') else {
        return false;
    };
    let symbol = &line[..open];
    !symbol.is_empty()
        && symbol.contains('.')
        && symbol
            .chars()
            .all(|character| is_ascii_alnum(character) || "._/*".contains(character))
}

fn is_dotnet_frame(text: &str) -> bool {
    text.starts_with("at ") && text.contains(") in ") && text.contains(":line ")
}

fn is_dotnet_exception_head(trimmed: &str) -> bool {
    let Some(colon) = trimmed.find(':') else {
        return false;
    };
    let head = &trimmed[..colon];
    head.ends_with("Exception")
        && head.contains('.')
        && head
            .chars()
            .all(|character| is_ascii_alnum(character) || "._`+".contains(character))
}

pub(super) fn is_rust_backtrace_frame(text: &str) -> bool {
    let trimmed = text.trim_start();
    let digits = leading_digits(trimmed);
    if digits == 0 || trimmed.as_bytes().get(digits) != Some(&b':') {
        return false;
    }
    let rest = trimmed[digits + 1..].trim_start_matches(' ');
    rest.strip_prefix("0x")
        .is_some_and(|hex| hex.starts_with(|character: char| character.is_ascii_hexdigit()))
}

fn is_java_more_summary(trimmed: &str) -> bool {
    trimmed.strip_prefix("... ").is_some_and(|rest| {
        let digits = leading_digits(rest);
        digits > 0 && rest[digits..].trim() == "more"
    })
}

pub(super) fn trace_flavor_for(line: &str) -> Option<Flavor> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("Traceback (most recent call last)") || is_python_file_frame(trimmed) {
        return Some(Flavor::Python);
    }
    // Before JavaScript and Java: a .NET frame also has their shape.
    if trimmed.starts_with("Unhandled exception.") || is_dotnet_frame(trimmed) {
        return Some(Flavor::Dotnet);
    }
    if is_js_at_frame(trimmed) {
        return Some(Flavor::JavaScript);
    }
    if is_java_at_frame(trimmed) {
        return Some(Flavor::Java);
    }
    if trimmed.starts_with("--> ") && has_line_col_suffix(trimmed) {
        return Some(Flavor::RustError);
    }
    if (trimmed.starts_with("thread '") && trimmed.contains("panicked at"))
        || trimmed.starts_with("stack backtrace:")
        || is_rust_backtrace_frame(line)
    {
        return Some(Flavor::RustBacktrace);
    }
    is_go_panic_opener(line).then_some(Flavor::GoPanic)
}

fn trace_terminates(flavor: Flavor, line: &str, lines_so_far: usize) -> bool {
    let trimmed = line.trim_start();
    let indented = line.starts_with(' ') || line.starts_with('\t');
    match flavor {
        Flavor::Python => {
            let continuation = [
                "Traceback",
                "File ",
                "During handling",
                "The above exception",
            ]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix));
            if indented || line.is_empty() || continuation {
                return false;
            }
            !trimmed.starts_with(|character: char| character.is_ascii_uppercase())
        }
        Flavor::JavaScript => !trimmed.starts_with("at ") && !line.is_empty(),
        Flavor::Java => {
            let chain = trimmed.starts_with("Caused by:")
                || trimmed.starts_with("Suppressed:")
                || is_java_more_summary(trimmed);
            !trimmed.starts_with("at ") && !chain && !line.is_empty()
        }
        Flavor::Dotnet => {
            !line.is_empty()
                && !(trimmed.starts_with("at ")
                    || trimmed.starts_with("--->")
                    || trimmed.starts_with("--- End of")
                    || is_dotnet_exception_head(trimmed))
        }
        Flavor::RustError => !trimmed.starts_with("--> ") && !line.is_empty(),
        Flavor::RustBacktrace => {
            if line.is_empty() || lines_so_far == 1 {
                return false;
            }
            let frame = trimmed.starts_with(|character: char| character.is_ascii_digit());
            let continuation = indented
                || trimmed.starts_with("stack backtrace:")
                || trimmed.starts_with("note: run with");
            !frame && !continuation
        }
        Flavor::GoPanic => {
            !line.is_empty()
                && !(line.starts_with('\t')
                    || is_goroutine_header(line)
                    || is_go_call_frame(line)
                    || line.starts_with("panic: ")
                    || line.starts_with("fatal error: ")
                    || line.starts_with("[signal "))
        }
    }
}

fn score(line: &LogLine<'_>) -> f64 {
    let base = match line.level {
        Level::Error | Level::Fail => 1.0,
        Level::Warn => 0.5,
        Level::Info | Level::Unknown => 0.1,
        Level::Debug => 0.05,
        Level::Trace => 0.02,
    };
    let stack = if line.is_stack { 0.3 } else { 0.0 };
    let summary = if line.is_summary { 0.4 } else { 0.0 };
    f64::min(base + stack + summary, 1.0)
}

/// Classifies every line: its level, whether it is a summary, and whether
/// it belongs to a stack trace (a trace ends where its flavor says or after
/// 20 lines).
pub(super) fn parse_log_lines<'a>(lines: &[&'a str]) -> Vec<LogLine<'a>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut active: Option<Flavor> = None;
    let mut trace_lines = 0;
    for (index, line) in lines.iter().copied().enumerate() {
        let mut entry = LogLine {
            index,
            content: Cow::Borrowed(line),
            level: classify_level(line),
            is_stack: false,
            is_summary: is_log_summary_line(line),
            score: 0.0,
        };
        match active {
            Some(flavor)
                if trace_lines < LOG_STACK_MAX_LINES
                    && !trace_terminates(flavor, line, trace_lines) =>
            {
                entry.is_stack = true;
                trace_lines += 1;
            }
            Some(flavor) => {
                let cap_hit = trace_lines >= LOG_STACK_MAX_LINES;
                active = None;
                trace_lines = 0;
                if let Some(next) = trace_flavor_for(line) {
                    active = Some(next);
                    trace_lines = 1;
                    entry.is_stack = true;
                } else if cap_hit && !trace_terminates(flavor, line, 2) {
                    // A trace capped at this line goes on: it reopens here.
                    active = Some(flavor);
                    trace_lines = 1;
                    entry.is_stack = true;
                }
            }
            None => {
                if let Some(flavor) = trace_flavor_for(line) {
                    active = Some(flavor);
                    trace_lines = 1;
                    entry.is_stack = true;
                }
            }
        }
        entry.score = score(&entry);
        out.push(entry);
    }
    out
}
