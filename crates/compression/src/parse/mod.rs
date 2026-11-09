// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/parse-compress-input.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
//
// Models and gateways do not always send strict JSON for `compress`: fenced
// JSON, trailing commas, raw newlines in strings, the whole argument
// stringified, single quotes, one entry object where the array belongs, a
// stream cut off mid-argument, or the line form in one string. One lenient
// parser takes them all; for a truncated stream it salvages the complete
// entries and never guesses a partial one. Diagnostics are data.

mod lenient;
mod line_form;

use serde_json::{Map, Value};

pub use line_form::derive_topic_from_summary;

use crate::text::{clamp_prefix, utf16_len};
use crate::RangeSpec;
use lenient::{
    looks_truncated, normalize_single_quotes, salvage_content_entries, strip_fence,
    try_parse_lenient,
};
use line_form::{
    coalesce_line_entries, is_headerless, parse_line_entry, split_line_entries,
    strip_json_wrapper_residue,
};

/// The pointer text a relocated `compress` call's arguments were rewritten
/// to; a model copying it as a new call echoes a fold already done.
const RELOCATED_POINTER_MARKER: &str = "[bili] compression succeeded — summary relocated";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseKind {
    Ok,
    EmptyInput,
    NotObject,
    MissingContent,
    ContentNotArray,
    AlreadyFoldedEcho,
    MalformedJson,
    Truncated,
    NoValidRanges,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseDiagnostics {
    /// At least one range was recovered.
    pub ok: bool,
    pub kind: ParseKind,
    /// The parse that won converted single quotes.
    pub quote_salvage: bool,
    /// A single range's summary came in the `content` string beside
    /// top-level refs.
    pub flat_summary_salvage: bool,
    /// A non-array `content` (one entry, or a nested `ranges` array) was
    /// taken as the array.
    pub content_salvage: bool,
    /// The first 800 characters of a string input.
    pub raw_prefix: Option<String>,
    /// A string input's length in UTF-16 code units.
    pub length: Option<usize>,
    /// The parsed object's top-level keys.
    pub keys: Vec<String>,
    /// Entries dropped as invalid, with one reason each.
    pub invalid_items: usize,
    pub invalid_reasons: Vec<String>,
}

impl ParseDiagnostics {
    fn new() -> Self {
        Self {
            ok: false,
            kind: ParseKind::Ok,
            quote_salvage: false,
            flat_summary_salvage: false,
            content_salvage: false,
            raw_prefix: None,
            length: None,
            keys: Vec::new(),
            invalid_items: 0,
            invalid_reasons: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCompressArgs {
    pub ranges: Vec<RangeSpec>,
    pub diagnostics: ParseDiagnostics,
}

/// The ranges of a `compress` call's arguments, as a decoded object or a
/// JSON string in any of the shapes above. Invalid entries are dropped and
/// counted; `call_id` becomes each range's `compress_call_id`.
pub fn parse_compress_args(input: &Value, call_id: Option<&str>) -> ParsedCompressArgs {
    let mut diagnostics = ParseDiagnostics::new();
    match input {
        Value::Null => {
            diagnostics.kind = ParseKind::EmptyInput;
            finish(Vec::new(), diagnostics)
        }
        Value::String(raw) => parse_string_input(raw, call_id, diagnostics),
        Value::Object(object) => parse_object(object, call_id, diagnostics),
        _ => {
            diagnostics.kind = ParseKind::NotObject;
            finish(Vec::new(), diagnostics)
        }
    }
}

fn parse_string_input(
    raw: &str,
    call_id: Option<&str>,
    mut diagnostics: ParseDiagnostics,
) -> ParsedCompressArgs {
    diagnostics.raw_prefix = Some(clamp_prefix(raw, 800).to_owned());
    diagnostics.length = Some(utf16_len(raw));
    let cleaned = strip_fence(raw.trim());
    let fresh = diagnostics.clone();
    let first = parse_string_core(cleaned, call_id, diagnostics);
    // Weak models send single-quoted arguments; the retry wins only when it
    // recovers more, so valid input is never rewritten.
    if first.ranges.is_empty() || first.diagnostics.invalid_items > 0 {
        if let Some(normalized) = normalize_single_quotes(cleaned) {
            let mut retry = parse_string_core(&normalized, call_id, fresh);
            if retry.ranges.len() > first.ranges.len() {
                retry.diagnostics.quote_salvage = true;
                return retry;
            }
        }
    }
    first
}

fn parse_string_core(
    cleaned: &str,
    call_id: Option<&str>,
    mut diagnostics: ParseDiagnostics,
) -> ParsedCompressArgs {
    if cleaned.is_empty() {
        diagnostics.kind = ParseKind::EmptyInput;
        return finish(Vec::new(), diagnostics);
    }
    let mut value = try_parse_lenient(cleaned);
    // One level of double stringification.
    if let Some(Value::String(inner)) = &value {
        if let Some(parsed) = try_parse_lenient(strip_fence(inner)) {
            value = Some(parsed);
        }
    }
    match value {
        Some(Value::Object(object)) => parse_object(&object, call_id, diagnostics),
        Some(_) => {
            diagnostics.kind = ParseKind::NotObject;
            finish(Vec::new(), diagnostics)
        }
        None => {
            let entries = salvage_content_entries(cleaned);
            let truncated = !entries.is_empty() || looks_truncated(cleaned);
            let (ranges, invalid, reasons) = validate_entries(entries, call_id);
            diagnostics.invalid_items = invalid;
            diagnostics.invalid_reasons = reasons;
            diagnostics.kind = if truncated {
                ParseKind::Truncated
            } else {
                ParseKind::MalformedJson
            };
            finish(ranges, diagnostics)
        }
    }
}

fn parse_object(
    object: &Map<String, Value>,
    call_id: Option<&str>,
    mut diagnostics: ParseDiagnostics,
) -> ParsedCompressArgs {
    diagnostics.keys = object.keys().cloned().collect();
    let (entries, salvaged) = match object.get("content") {
        // One range at the top level.
        None => {
            return match validate_entry(&Value::Object(object.clone()), call_id) {
                Ok(range) => {
                    diagnostics.kind = ParseKind::Ok;
                    finish(vec![range], diagnostics)
                }
                Err(_) => {
                    diagnostics.kind = ParseKind::MissingContent;
                    finish(Vec::new(), diagnostics)
                }
            };
        }
        Some(Value::Array(entries)) => (entries.clone(), false),
        Some(Value::String(content)) => {
            if content.contains(RELOCATED_POINTER_MARKER) {
                diagnostics.kind = ParseKind::AlreadyFoldedEcho;
                return finish(Vec::new(), diagnostics);
            }
            match parse_content_array(content) {
                Some(parsed) if !parsed.entries.is_empty() || content.trim().is_empty() => {
                    if parsed.quote_repaired {
                        diagnostics.quote_salvage = true;
                    }
                    (parsed.entries, parsed.salvaged)
                }
                _ => return parse_bare_content(object, content, call_id, diagnostics),
            }
        }
        // One entry object where the array belongs, or a nested `ranges`.
        Some(Value::Object(entry)) => {
            diagnostics.content_salvage = true;
            match entry.get("ranges") {
                Some(Value::Array(nested)) => (nested.clone(), false),
                _ => (vec![Value::Object(entry.clone())], false),
            }
        }
        Some(_) => {
            diagnostics.kind = ParseKind::ContentNotArray;
            return finish(Vec::new(), diagnostics);
        }
    };

    let (mut ranges, invalid, reasons) = validate_entries(entries, call_id);
    // A top-level topic and summary cap apply to the ranges without their own.
    let top_topic = string_or(object.get("topic"));
    let top_max = object
        .get("summaryMaxChars")
        .and_then(Value::as_f64)
        .filter(|max| max.is_finite());
    for range in &mut ranges {
        if range.topic.is_none() {
            range.topic.clone_from(&top_topic);
        }
        if range.summary_max_chars.is_none() {
            range.summary_max_chars = top_max.map(|max| max as usize);
        }
    }
    diagnostics.invalid_items = invalid;
    diagnostics.invalid_reasons = reasons;
    diagnostics.kind = if salvaged {
        ParseKind::Truncated
    } else if ranges.is_empty() {
        ParseKind::NoValidRanges
    } else {
        ParseKind::Ok
    };
    finish(ranges, diagnostics)
}

/// A `content` string that is not a JSON array: the line form, once the
/// residue of a broken stringified array is gone; else a flat range's
/// summary or one stringified entry.
fn parse_bare_content(
    object: &Map<String, Value>,
    content: &str,
    call_id: Option<&str>,
    mut diagnostics: ParseDiagnostics,
) -> ParsedCompressArgs {
    let entries: Vec<Value> = split_line_entries(&strip_json_wrapper_residue(content))
        .into_iter()
        .map(Value::String)
        .collect();
    let all_headerless = entries.iter().all(is_headerless);
    let (ranges, invalid, reasons) = validate_entries(entries.clone(), call_id);
    if !ranges.is_empty() {
        diagnostics.invalid_items = invalid;
        diagnostics.invalid_reasons = reasons;
        diagnostics.kind = ParseKind::Ok;
        return finish(ranges, diagnostics);
    }
    if all_headerless {
        if let Some(flat) = flat_single_range(object, content, call_id) {
            diagnostics.flat_summary_salvage = true;
            diagnostics.kind = ParseKind::Ok;
            return finish(vec![flat], diagnostics);
        }
        if let Some(single) = stringified_entry(content, call_id) {
            diagnostics.content_salvage = true;
            diagnostics.kind = ParseKind::Ok;
            return finish(vec![single], diagnostics);
        }
    }
    diagnostics.invalid_items = invalid;
    diagnostics.invalid_reasons = reasons;
    // Refs headers that failed validation were a line-form attempt; prose
    // never was.
    diagnostics.kind = if all_headerless {
        ParseKind::ContentNotArray
    } else {
        ParseKind::NoValidRanges
    };
    finish(Vec::new(), diagnostics)
}

/// The summary of one range in the `content` string, with its refs at the
/// top level; a JSON `{summary, topic}` in the string is unwrapped first.
fn flat_single_range(
    object: &Map<String, Value>,
    content: &str,
    call_id: Option<&str>,
) -> Option<RangeSpec> {
    let field = |names: [&str; 3]| {
        names
            .into_iter()
            .find_map(|name| string_or(object.get(name)))
    };
    let start = field(["startRef", "startId", "messageId"])?;
    let end = field(["endRef", "endId", "messageId"])?;
    let mut summary = string_or(object.get("summary"));
    let mut topic = string_or(object.get("topic"));
    if summary.is_none() {
        if let Some(Value::Object(inner)) = try_parse_lenient(strip_fence(content.trim())) {
            summary = string_or(inner.get("summary"));
            if topic.is_none() {
                topic = string_or(inner.get("topic"));
            }
        }
        if summary.is_none() && !content.trim().is_empty() {
            summary = Some(content.trim().to_owned());
        }
    }
    let mut entry = Map::new();
    entry.insert("startRef".to_owned(), Value::String(start));
    entry.insert("endRef".to_owned(), Value::String(end));
    entry.insert("summary".to_owned(), Value::String(summary?));
    if let Some(topic) = topic {
        entry.insert("topic".to_owned(), Value::String(topic));
    }
    validate_entry(&Value::Object(entry), call_id).ok()
}

/// The whole `content` string as one stringified entry object.
fn stringified_entry(content: &str, call_id: Option<&str>) -> Option<RangeSpec> {
    let parsed @ Value::Object(_) = try_parse_lenient(strip_fence(content.trim()))? else {
        return None;
    };
    validate_entry(&parsed, call_id).ok()
}

struct ContentArray {
    entries: Vec<Value>,
    salvaged: bool,
    quote_repaired: bool,
}

/// A stringified `content` array; a truncated one yields its complete
/// entries; a single-quoted one is retried once.
fn parse_content_array(s: &str) -> Option<ContentArray> {
    let cleaned = strip_fence(s.trim());
    let direct = parse_content_array_core(cleaned);
    if direct
        .as_ref()
        .is_some_and(|direct| !direct.entries.is_empty())
    {
        return direct;
    }
    if let Some(normalized) = normalize_single_quotes(cleaned) {
        if let Some(mut retry) =
            parse_content_array_core(&normalized).filter(|retry| !retry.entries.is_empty())
        {
            retry.quote_repaired = true;
            return Some(retry);
        }
    }
    direct
}

fn parse_content_array_core(s: &str) -> Option<ContentArray> {
    let mut value = if s.is_empty() {
        None
    } else {
        try_parse_lenient(s)
    };
    if let Some(Value::String(inner)) = &value {
        value = try_parse_lenient(strip_fence(inner));
    }
    match value {
        Some(Value::Array(entries)) => Some(ContentArray {
            entries,
            salvaged: false,
            quote_repaired: false,
        }),
        None => {
            let entries = salvage_content_entries(&format!("{{\"content\": {s}"));
            Some(ContentArray {
                salvaged: !entries.is_empty(),
                entries,
                quote_repaired: false,
            })
        }
        Some(_) => None,
    }
}

fn finish(ranges: Vec<RangeSpec>, mut diagnostics: ParseDiagnostics) -> ParsedCompressArgs {
    diagnostics.ok = !ranges.is_empty();
    ParsedCompressArgs {
        ranges,
        diagnostics,
    }
}

fn validate_entries(
    entries: Vec<Value>,
    call_id: Option<&str>,
) -> (Vec<RangeSpec>, usize, Vec<String>) {
    let mut ranges = Vec::new();
    let mut reasons = Vec::new();
    for (index, entry) in coalesce_line_entries(entries).iter().enumerate() {
        match validate_entry(entry, call_id) {
            Ok(range) => ranges.push(range),
            Err(reason) => reasons.push(format!("entry {index}: {reason}")),
        }
    }
    let invalid = reasons.len();
    (ranges, invalid, reasons)
}

/// One entry: a line-form string, or an object with `startRef`/`startId`
/// (or `messageId`), `endRef`/`endId` (or `messageId`), `summary`, and
/// optionally `topic` and `summaryMaxChars`.
fn validate_entry(entry: &Value, call_id: Option<&str>) -> Result<RangeSpec, String> {
    let object = match entry {
        Value::String(text) => return parse_line_entry(text, call_id),
        Value::Object(object) => object,
        _ => return Err("not an object".to_owned()),
    };
    let field = |names: [&str; 3]| {
        names
            .into_iter()
            .find_map(|name| string_or(object.get(name)))
    };
    let (Some(start_ref), Some(end_ref)) = (
        field(["startRef", "startId", "messageId"]),
        field(["endRef", "endId", "messageId"]),
    ) else {
        return Err("missing range bounds (need startRef/startId and endRef/endId)".to_owned());
    };
    let summary = string_or(object.get("summary")).ok_or_else(|| "missing summary".to_owned())?;
    Ok(RangeSpec {
        start_ref,
        end_ref,
        summary,
        topic: string_or(object.get("topic")),
        compress_call_id: call_id.map(str::to_owned),
        summary_max_chars: object
            .get("summaryMaxChars")
            .and_then(Value::as_f64)
            .filter(|max| max.is_finite())
            .map(|max| max as usize),
    })
}

fn string_or(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}
