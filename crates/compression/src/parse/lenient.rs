// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the tolerant JSON parsing of `kernel/src/parse-compress-input.ts` of
// acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026 ranxianglei.
// MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: control characters and bad escapes inside
// strings are repaired by one scan that tracks strings as the JSON grammar
// does, where the kernel repaired them at the positions `JSON.parse`
// reported one error at a time.

use serde_json::Value;

/// `s` as JSON, else with trailing commas dropped, else with raw control
/// characters and bad escapes inside strings repaired.
pub(super) fn try_parse_lenient(s: &str) -> Option<Value> {
    if s.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str(s) {
        return Some(value);
    }
    let without_commas = strip_trailing_commas(s);
    if without_commas != s {
        if let Ok(value) = serde_json::from_str(&without_commas) {
            return Some(value);
        }
    }
    let escaped = escape_raw_newlines_in_strings(&without_commas);
    if escaped != without_commas {
        if let Ok(value) = serde_json::from_str(&escaped) {
            return Some(value);
        }
    }
    // Unescaped quotes and markdown escapes (\`, \-) defeat the scan above.
    let repaired = repair_strings(&without_commas);
    if repaired != without_commas {
        if let Ok(value) = serde_json::from_str(&repaired) {
            return Some(value);
        }
    }
    None
}

/// The body of a Markdown code fence, or `s` unchanged.
pub(super) fn strip_fence(s: &str) -> &str {
    if !s.starts_with("```") {
        return s;
    }
    let Some(newline) = s.find('\n') else {
        return s;
    };
    let body = newline + 1;
    match s.rfind("```") {
        Some(end) if end > body => s[body..end].trim(),
        _ => s[body..].trim(),
    }
}

/// Converts single-quoted strings to double-quoted ones; an apostrophe in a
/// double-quoted string is data. `None` when nothing was converted. It
/// never invents structure.
pub(super) fn normalize_single_quotes(raw: &str) -> Option<String> {
    if !raw.contains('\'') || (!raw.contains('{') && !raw.contains('[')) {
        return None;
    }
    let mut out = String::with_capacity(raw.len());
    let mut changed = false;
    let mut in_double = false;
    let mut in_single = false;
    let mut chars = raw.chars();
    while let Some(character) = chars.next() {
        if in_double {
            out.push(character);
            if character == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if character == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            match character {
                '\\' => match chars.next() {
                    Some('\'') => out.push('\''),
                    Some(next) => {
                        out.push('\\');
                        out.push(next);
                    }
                    None => out.push('\\'),
                },
                '\'' => {
                    out.push('"');
                    in_single = false;
                    changed = true;
                }
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ => out.push(character),
            }
            continue;
        }
        match character {
            '"' => {
                in_double = true;
                out.push(character);
            }
            '\'' => {
                in_single = true;
                out.push('"');
                changed = true;
            }
            _ => out.push(character),
        }
    }
    changed.then_some(out)
}

/// Drops commas right before a closing brace or bracket, outside strings.
fn strip_trailing_commas(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in s.char_indices() {
        if in_string {
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
        } else if character == ',' {
            let next = s[index + 1..]
                .trim_start_matches([' ', '\t', '\n', '\r'])
                .chars()
                .next();
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        out.push(character);
    }
    out
}

/// Escapes raw newlines, carriage returns and tabs inside strings.
fn escape_raw_newlines_in_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut escaped = false;
    for character in s.chars() {
        if !in_string {
            if character == '"' {
                in_string = true;
            }
            out.push(character);
            continue;
        }
        if escaped {
            out.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' => {
                out.push(character);
                escaped = true;
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => {
                in_string = false;
                out.push(character);
            }
            _ => out.push(character),
        }
    }
    out
}

/// Inside strings: escapes every control character and doubles a backslash
/// that starts no valid escape, so the text after it stays literal.
fn repair_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut chars = s.chars().peekable();
    while let Some(character) = chars.next() {
        if !in_string {
            if character == '"' {
                in_string = true;
            }
            out.push(character);
            continue;
        }
        match character {
            '"' => {
                in_string = false;
                out.push(character);
            }
            '\\' => match chars.peek() {
                Some(next @ ('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u')) => {
                    out.push('\\');
                    out.push(*next);
                    chars.next();
                }
                _ => out.push_str("\\\\"),
            },
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32))
            }
            _ => out.push(character),
        }
    }
    out
}

/// An open bracket or string at the end: the signature of a cut-off stream.
pub(super) fn looks_truncated(s: &str) -> bool {
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut escaped = false;
    for character in s.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth > 0 || in_string
}

/// The complete entries of a truncated `content` array: every object that
/// opens and closes at its top level and still parses. A partial entry is
/// dropped, never guessed.
pub(super) fn salvage_content_entries(raw: &str) -> Vec<Value> {
    let Some(array_start) = content_array_start(raw) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut entry_start: Option<usize> = None;
    for (offset, character) in raw[array_start + 1..].char_indices() {
        let index = array_start + 1 + offset;
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' | '[' => {
                if depth == 0 && character == '{' && entry_start.is_none() {
                    entry_start = Some(index);
                }
                depth += 1;
            }
            '}' | ']' => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
                if depth == 0 {
                    if let Some(start) = entry_start.take() {
                        if let Some(entry) = try_parse_lenient(&raw[start..=index]) {
                            entries.push(entry);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    entries
}

/// The position of the `[` in the first `"content" : [`.
fn content_array_start(raw: &str) -> Option<usize> {
    let key = "\"content\"";
    let mut from = 0;
    while let Some(found) = raw[from..].find(key) {
        let after_key = from + found + key.len();
        let rest = raw[after_key..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest_trimmed = rest.trim_start();
            if rest_trimmed.starts_with('[') {
                return Some(raw.len() - rest_trimmed.len());
            }
        }
        from = after_key;
    }
    None
}
