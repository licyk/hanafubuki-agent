// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the tool patterns of `kernel/src/protected.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: a tool's path is its name; there is no
// projection of skill loads onto `skill/<name>`.

use super::NEVER_PRESERVE_RECENT_TOOLS;
use crate::model::Message;

/// An exact name or a trailing-`*` prefix glob, without regard to case.
pub fn match_tool_pattern(tool_name: &str, pattern: &str) -> bool {
    let name = tool_name.to_lowercase();
    let pattern = pattern.to_lowercase();
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// A `/`-separated tool path against a pattern of the same shape; each
/// segment matches as [`match_tool_pattern`] (`*` never crosses `/`), and a
/// one-segment pattern matches the first segment and everything under it.
pub fn match_tool_path(path: &str, pattern: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let path = path.to_lowercase();
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    let path_segments: Vec<&str> = path.split('/').collect();
    let pattern_first = pattern_segments.first().copied().unwrap_or_default();
    let path_first = path_segments.first().copied().unwrap_or_default();
    if pattern_segments.len() == 1 || (pattern_segments.len() == 2 && pattern_segments[1] == "*") {
        return match_tool_pattern(path_first, pattern_first);
    }
    pattern_segments.len() == path_segments.len()
        && pattern_segments
            .iter()
            .zip(&path_segments)
            .all(|(segment, path_segment)| match_tool_pattern(path_segment, segment))
}

/// A protection pattern against a tool exchange, by path or by bare name.
pub(crate) fn match_tool_message_pattern(message: &Message, pattern: &str) -> bool {
    let Some(tool_name) = message.tool_name.as_deref().filter(|name| !name.is_empty()) else {
        return false;
    };
    match_tool_path(tool_name, pattern) || match_tool_pattern(tool_name, pattern)
}

/// A tool call or result that does not count toward the recent zone:
/// `patterns` (the built-in list when `None`) less `preserve_patterns`.
pub fn is_never_preserve_recent(
    message: &Message,
    patterns: Option<&[String]>,
    preserve_patterns: &[String],
) -> bool {
    if !message.is_tool() {
        return false;
    }
    let Some(tool_name) = message.tool_name.as_deref().filter(|name| !name.is_empty()) else {
        return false;
    };
    let built_in: Vec<String>;
    let base = match patterns {
        Some(patterns) => patterns,
        None => {
            built_in = NEVER_PRESERVE_RECENT_TOOLS.map(str::to_owned).to_vec();
            &built_in
        }
    };
    base.iter()
        .filter(|tool| {
            !preserve_patterns
                .iter()
                .any(|preserve| match_tool_pattern(tool, preserve))
        })
        .any(|pattern| match_tool_pattern(tool_name, pattern))
}
