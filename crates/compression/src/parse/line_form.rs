// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the line form of `kernel/src/parse-compress-input.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the header patterns are matched by hand.
//
// A line-form entry is one string: the first line `m00150–m00220 topic`,
// the rest the summary. Refs must open the first line, since summaries cite
// refs all the time.

use serde_json::Value;

use crate::text::{clamp_prefix, utf16_len};
use crate::RangeSpec;

const PAIR_SEPARATORS: [char; 7] = [
    '-', '\u{2013}', '\u{2014}', '\u{2026}', '~', '\u{2212}', '\u{ff0d}',
];

/// The end of `[mb]` and one to seven digits at `at`, the digits followed by
/// a non-digit.
fn ref_token(s: &str, at: usize) -> Option<usize> {
    let rest = s.get(at..)?;
    let mut chars = rest.char_indices();
    let (_, prefix) = chars.next()?;
    if !matches!(prefix, 'm' | 'M' | 'b' | 'B') {
        return None;
    }
    let digits = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
    (1..=7).contains(&digits).then_some(at + 1 + digits)
}

/// A ref token at `at` that ends on an ASCII word boundary.
fn bounded_ref_token(s: &str, at: usize) -> Option<usize> {
    let end = ref_token(s, at)?;
    let next = s[end..].chars().next();
    (!next.is_some_and(|next| next.is_ascii_alphanumeric() || next == '_')).then_some(end)
}

fn skip_whitespace(s: &str, at: usize) -> usize {
    let rest = &s[at..];
    at + (rest.len() - rest.trim_start().len())
}

/// The end of a pair separator at `at`: one dash-like character, `..`,
/// `...` or `to`.
fn separator(s: &str, at: usize) -> Option<usize> {
    let rest = &s[at..];
    let first = rest.chars().next()?;
    if PAIR_SEPARATORS.contains(&first) {
        return Some(at + first.len_utf8());
    }
    if first == '.' {
        let dots = rest.bytes().take_while(|byte| *byte == b'.').count();
        return (2..=3).contains(&dots).then_some(at + dots);
    }
    let pair = rest.get(..2)?;
    pair.eq_ignore_ascii_case("to").then_some(at + 2)
}

/// A ref pair at `at`: the two refs and where the match ends.
pub(super) fn ref_pair_at(s: &str, at: usize) -> Option<(&str, &str, usize)> {
    let first_end = ref_token(s, at)?;
    let after_separator = separator(s, skip_whitespace(s, first_end))?;
    let second_start = skip_whitespace(s, after_separator);
    let second_end = bounded_ref_token(s, second_start)?;
    Some((&s[at..first_end], &s[second_start..second_end], second_end))
}

/// The line opens with refs, a pair or a single one.
fn has_header_ref(head: &str) -> bool {
    ref_pair_at(head, 0).is_some() || bounded_ref_token(head, 0).is_some()
}

/// A second ref behind a separator the pair pattern does not know: one to
/// three symbols, then a ref.
fn unrecognized_separator(rest: &str) -> bool {
    let start = skip_whitespace(rest, 0);
    let symbols = rest[start..]
        .chars()
        .take_while(|character| {
            !character.is_alphabetic() && !character.is_numeric() && !character.is_whitespace()
        })
        .take(4)
        .collect::<Vec<_>>();
    if symbols.is_empty() || symbols.len() > 3 {
        return false;
    }
    let after: usize = start
        + symbols
            .iter()
            .map(|character| character.len_utf8())
            .sum::<usize>();
    bounded_ref_token(rest, skip_whitespace(rest, after)).is_some()
}

fn normalize_line_ref(raw: &str) -> String {
    let lower = raw.to_lowercase();
    let digits = &lower[1..];
    if lower.starts_with('b') {
        format!("b{digits}")
    } else {
        format!("m{digits:0>5}")
    }
}

fn split_head(entry: &str) -> (&str, &str) {
    match entry.find('\n') {
        Some(newline) => (entry[..newline].trim(), entry[newline + 1..].trim()),
        None => (entry.trim(), ""),
    }
}

/// A string entry without any refs header: prose.
pub(super) fn is_headerless(entry: &Value) -> bool {
    entry
        .as_str()
        .is_some_and(|text| !has_header_ref(split_head(text).0))
}

pub(super) fn parse_line_entry(entry: &str, call_id: Option<&str>) -> Result<RangeSpec, String> {
    let (head, summary) = split_head(entry);
    let (start_ref, end_ref, after_refs) = match ref_pair_at(head, 0) {
        Some((start, end, after)) => (normalize_line_ref(start), normalize_line_ref(end), after),
        None => {
            let Some(single_end) = bounded_ref_token(head, 0) else {
                return Err("line entry: no mNNNNN/bN refs in header".to_owned());
            };
            // Narrowing a pair behind an unknown separator to its first ref
            // would fold one message of a many-message range.
            if unrecognized_separator(&head[single_end..]) {
                return Err("line entry: refs separator not recognized — refusing to narrow a ref pair to its first ref (use -, –, —, …, ~, .., or to)".to_owned());
            }
            let single = normalize_line_ref(&head[..single_end]);
            (single.clone(), single, single_end)
        }
    };
    if summary.is_empty() {
        return Err("line entry: missing summary after the refs header line (every entry needs refs THEN its summary text on the same or the next line; a note about an already-folded range does not belong in the batch — drop it; to rewrite an existing block use a bN..bM block-id header entry)".to_owned());
    }
    let explicit_topic = head[after_refs..].trim();
    let topic = if explicit_topic.is_empty() {
        derive_topic_from_summary(summary)
    } else {
        Some(explicit_topic.to_owned())
    };
    Ok(RangeSpec {
        start_ref,
        end_ref,
        summary: summary.to_owned(),
        topic,
        compress_call_id: call_id.map(str::to_owned),
        summary_max_chars: None,
    })
}

/// A label for an entry without a topic: the summary's first Markdown
/// heading, else its first non-empty line, cut to 60 characters.
pub fn derive_topic_from_summary(summary: &str) -> Option<String> {
    let source = match first_heading(summary) {
        Some(heading) => heading,
        None => summary
            .split('\n')
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default(),
    };
    let mut text = source.trim();
    for bullet in ['\u{2022}', '-', '*'] {
        if let Some(rest) = text.strip_prefix(bullet) {
            if rest.starts_with(char::is_whitespace) {
                text = rest.trim_start();
            }
            break;
        }
    }
    if text.is_empty() {
        return None;
    }
    if utf16_len(text) > 60 {
        return Some(clamp_prefix(text, 60).trim_end().to_owned());
    }
    Some(text.to_owned())
}

/// The text of the first line holding `#{1,6}`, whitespace, then text, as
/// the multiline pattern `^#{1,6}\s+(.+)$` finds it (its whitespace may run
/// over line ends).
fn first_heading(summary: &str) -> Option<&str> {
    let line_starts = std::iter::once(0).chain(
        summary
            .char_indices()
            .filter(|(_, character)| is_line_end(*character))
            .map(|(index, character)| index + character.len_utf8()),
    );
    for start in line_starts {
        let line = &summary[start..];
        let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
        if !(1..=6).contains(&hashes) {
            continue;
        }
        let after = &line[hashes..];
        let text_start = after.len() - after.trim_start().len();
        if text_start == 0 {
            continue;
        }
        let text = &after[text_start..];
        if text.is_empty() {
            // Only whitespace to the end: the pattern still matches a blank
            // heading (one that yields no topic) when a whitespace character
            // after the first ends a line.
            let run: Vec<char> = after.chars().collect();
            let blank = (1..run.len()).any(|j| {
                !is_line_end(run[j]) && run.get(j + 1).is_none_or(|next| is_line_end(*next))
            });
            return blank.then_some("");
        }
        let end = text.find(is_line_end).unwrap_or(text.len());
        return Some(&text[..end]);
    }
    None
}

fn is_line_end(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Splits a bare string `content` into line-form entries: a line that
/// starts a ref pair (behind the residue of a stringified array element, if
/// any) begins a new entry. The header test is a linear scan, once per
/// whitespace run, since the content is model-written.
pub(super) fn split_line_entries(content: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut newline = content.find('\n');
    while let Some(at) = newline {
        let run_end = skip_whitespace(content, at + 1);
        if starts_line_entry(content, run_end) {
            for (offset, byte) in content.as_bytes()[at..run_end].iter().enumerate() {
                if *byte == b'\n' {
                    chunks.push(&content[chunk_start..at + offset]);
                    chunk_start = at + offset + 1;
                }
            }
        }
        newline = content[run_end..].find('\n').map(|found| run_end + found);
    }
    chunks.push(&content[chunk_start..]);
    let parts: Vec<String> = chunks
        .into_iter()
        .map(|chunk| strip_element_residue(chunk.trim()))
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        vec![content.trim().to_owned()]
    } else {
        parts
    }
}

fn is_quote(character: Option<char>) -> bool {
    matches!(character, Some('"' | '\''))
}

/// `\s*(?:["']\s*,\s*)?["']?\s*<refs>` at `at`, which sits past the
/// leading whitespace.
fn starts_line_entry(s: &str, at: usize) -> bool {
    let char_at = |index: usize| s.get(index..).and_then(|rest| rest.chars().next());
    if !is_quote(char_at(at)) {
        return ref_pair_at(s, at).is_some();
    }
    let after_quote = skip_whitespace(s, at + 1);
    if char_at(after_quote) != Some(',') {
        return ref_pair_at(s, after_quote).is_some();
    }
    let after_comma = skip_whitespace(s, after_quote + 1);
    if !is_quote(char_at(after_comma)) {
        return ref_pair_at(s, after_comma).is_some();
    }
    ref_pair_at(s, skip_whitespace(s, after_comma + 1)).is_some()
}

/// Leading quotes and commas, and one trailing quote, left by a stringified
/// array element.
fn strip_element_residue(part: &str) -> String {
    let mut rest = part;
    while let Some(after) = rest.strip_prefix(['"', '\'']) {
        let after = after.trim_start();
        rest = after.strip_prefix(',').unwrap_or(after).trim_start();
    }
    let trimmed = rest.trim_end();
    match trimmed.strip_suffix(['"', '\'']) {
        Some(stripped) => stripped.to_owned(),
        None => rest.to_owned(),
    }
}

/// The shell of a stringified array whose escaping broke: leading `[` and a
/// quote, trailing `]` and a quote.
pub(super) fn strip_json_wrapper_residue(s: &str) -> String {
    let mut text = s.trim();
    if text.starts_with('[') {
        text = text.trim_start_matches('[').trim_start();
    }
    text = text.strip_prefix('"').unwrap_or(text);
    if text.ends_with(']') {
        text = text.trim_end_matches(']').trim_end();
    }
    text = text.strip_suffix('"').unwrap_or(text);
    text.trim().to_owned()
}

/// Joins a refs-only string entry with the refless string after it: models
/// split the header and its summary into sibling strings.
pub(super) fn coalesce_line_entries(entries: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(entries.len());
    let mut header_only: Option<usize> = None;
    for entry in entries {
        let Value::String(text) = &entry else {
            out.push(entry);
            header_only = None;
            continue;
        };
        let has_newline = text.contains('\n');
        let has_ref = has_header_ref(split_head(text).0);
        if let (false, Some(index)) = (has_ref, header_only) {
            if let Some(Value::String(header)) = out.get_mut(index) {
                header.push('\n');
                header.push_str(text);
                continue;
            }
        }
        let refs_only = has_ref && !has_newline;
        out.push(entry);
        header_only = refs_only.then(|| out.len() - 1);
    }
    out
}
