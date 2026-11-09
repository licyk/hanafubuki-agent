// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/ccr.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the retrieve tool's name is fixed, so a
// placeholder is built without one (a parsed one still reports the name it
// cites).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::StoredEntry;
use crate::prompts::RETRIEVE_TOOL_NAME;
use crate::text::{clamp_prefix, collapse_whitespace, utf16_len};

/// In every placeholder. A placeholder is recognised by its whole shape
/// ([`parse_stored_placeholder`]), never by finding this in a text.
pub const STORED_PLACEHOLDER_MARKER: &str = "[acp-stored";

const KIND_LABELS: [(&str, &str); 19] = [
    ("bash", "shell output"),
    ("shell", "shell output"),
    ("exec", "shell output"),
    ("execute_command", "shell output"),
    ("run", "shell output"),
    ("terminal", "shell output"),
    ("read", "file read"),
    ("read_file", "file read"),
    ("cat", "file read"),
    ("open", "file read"),
    ("grep", "search output"),
    ("rg", "search output"),
    ("search", "search output"),
    ("glob", "search output"),
    ("find", "search output"),
    ("webfetch", "web fetch"),
    ("web_fetch", "web fetch"),
    ("fetch", "web fetch"),
    ("curl", "web fetch"),
];

const COMMAND_FIELDS: [&str; 6] = ["command", "cmd", "script", "query", "path", "url"];

/// What a tool's output is, as a placeholder labels it.
pub fn classify_kind(tool_name: Option<&str>) -> &'static str {
    let Some(name) = tool_name else {
        return "tool result";
    };
    let name = name.to_lowercase();
    KIND_LABELS
        .iter()
        .find(|(tool, _)| *tool == name)
        .map_or("tool result", |(_, label)| label)
}

/// `text` on one line, whitespace runs collapsed, at most `max_chars`
/// UTF-16 code units and an ellipsis.
pub fn normalize_head(text: &str, max_chars: usize) -> String {
    cap(collapse_whitespace(text).trim(), max_chars)
}

/// The call's subject (its `command`, `cmd`, `script`, `query`, `path` or
/// `url`, the first that is a non-blank string) from its JSON arguments.
pub fn extract_command(arguments: Option<&str>, max_chars: usize) -> Option<String> {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(arguments?) else {
        return None;
    };
    COMMAND_FIELDS
        .iter()
        .find_map(|field| match fields.get(*field) {
            Some(Value::String(value)) if !value.trim().is_empty() => {
                Some(cap(collapse_whitespace(value).trim(), max_chars))
            }
            _ => None,
        })
}

fn cap(text: &str, max_chars: usize) -> String {
    if utf16_len(text) <= max_chars {
        text.to_owned()
    } else {
        format!("{}…", clamp_prefix(text, max_chars))
    }
}

/// The fields a placeholder is built from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredPlaceholder {
    pub r#ref: String,
    pub kind: String,
    pub tokens: u64,
    pub head: String,
    /// Shown instead of the head when there is one.
    pub command: Option<String>,
}

/// The two-line placeholder: the same fields always give the same bytes.
pub fn build_stored_placeholder(input: &StoredPlaceholder) -> String {
    let title = input.command.as_deref().unwrap_or(&input.head);
    let title = if title.is_empty() {
        String::new()
    } else {
        format!(" `{title}`")
    };
    format!(
        "📦 {STORED_PLACEHOLDER_MARKER} #{} · {} · {} tok]{title}\n   → {RETRIEVE_TOOL_NAME}(\"{}\") returns the full text",
        input.r#ref,
        input.kind,
        group_thousands(input.tokens),
        input.r#ref
    )
}

/// The placeholder a stored ref showed when it arrived, rebuilt from its
/// entry; an entry without a command takes it from the call's arguments.
pub fn restore_stored_placeholder_text(
    r#ref: &str,
    entry: &StoredEntry,
    call_arguments: Option<&str>,
    max_head_chars: usize,
) -> String {
    build_stored_placeholder(&StoredPlaceholder {
        r#ref: r#ref.to_owned(),
        kind: entry.kind.clone(),
        tokens: entry.tokens,
        head: entry.head.clone(),
        command: entry
            .command
            .clone()
            .or_else(|| extract_command(call_arguments, max_head_chars)),
    })
}

/// A recognised placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedStoredPlaceholder {
    pub r#ref: String,
    pub kind: String,
    pub tokens: u64,
    pub retrieve_tool_name: String,
    /// The command or head preview, when it has one.
    pub title: Option<String>,
}

// A canonical ref only (five digits, or six or seven without a leading
// zero), a JavaScript `.` (no line terminator), grouped token counts.
const REF: &str = "m(?:[0-9]{5}|[1-9][0-9]{5,6})";
const DOT: &str = "[^\n\r\u{2028}\u{2029}]";

static LINE1: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        "^📦 \\[acp-stored #({REF}) · ({DOT}+?) · (0|[1-9][0-9]{{0,2}}(?:,[0-9]{{3}})*) tok\\](?: `({DOT}+)`)?$"
    ))
    .ok()
});
static LINE2: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        "^   → ({DOT}+?)\\(\"({REF})\"\\) returns the full text$"
    ))
    .ok()
});
static LEADING_TAG: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new("^<acp [^>]*>[^<]*</acp>\n?").ok());

/// `text` without one leading `<acp>` ref tag.
pub fn strip_leading_tag(text: &str) -> &str {
    match LEADING_TAG.as_ref().and_then(|tag| tag.find(text)) {
        Some(found) => &text[found.end()..],
        None => text,
    }
}

/// The placeholder `text` is, as a whole (after one leading ref tag and
/// before one trailing newline); `None` for any other text, including one
/// that quotes a placeholder. A caller that knows the message's own ref
/// compares it before trusting the result.
pub fn parse_stored_placeholder(text: &str) -> Option<ParsedStoredPlaceholder> {
    let body = strip_leading_tag(text);
    let body = body.strip_suffix('\n').unwrap_or(body);
    let (line1, line2) = body.split_once('\n')?;
    if line1.is_empty() || line2.contains('\n') {
        return None;
    }
    let head = LINE1.as_ref()?.captures(line1)?;
    let hint = LINE2.as_ref()?.captures(line2)?;
    let r#ref = head.get(1)?.as_str();
    if hint.get(2)?.as_str() != r#ref {
        return None;
    }
    Some(ParsedStoredPlaceholder {
        r#ref: r#ref.to_owned(),
        kind: head.get(2)?.as_str().to_owned(),
        tokens: head.get(3)?.as_str().replace(',', "").parse().ok()?,
        retrieve_tool_name: hint.get(1)?.as_str().to_owned(),
        title: head.get(4).map(|title| title.as_str().to_owned()),
    })
}

/// Whether `text` is a placeholder as a whole: the tool result was already
/// replaced and is not stored, crushed or absorbed again.
pub fn is_stored_placeholder_text(text: &str) -> bool {
    parse_stored_placeholder(text).is_some()
}

/// `4,213`.
pub(super) fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
