// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `kernel/tests/parse-compress-input.test.ts`.

use hanafubuki_agent_compression::{parse_compress_args, ParseKind, ParsedCompressArgs};
use serde_json::{json, Value};

pub(super) fn parse(input: Value) -> ParsedCompressArgs {
    parse_compress_args(&input, None)
}

pub(super) fn parse_str(input: &str) -> ParsedCompressArgs {
    parse_compress_args(&Value::String(input.to_owned()), None)
}

pub(super) fn refs(parsed: &ParsedCompressArgs) -> Vec<(&str, &str)> {
    parsed
        .ranges
        .iter()
        .map(|range| (range.start_ref.as_str(), range.end_ref.as_str()))
        .collect()
}

pub(super) fn first_reason(parsed: &ParsedCompressArgs) -> &str {
    parsed
        .diagnostics
        .invalid_reasons
        .first()
        .map(String::as_str)
        .unwrap_or_default()
}

// Input-shape normalization.

#[test]
fn parses_a_valid_object_with_all_fields() {
    let parsed = parse(json!({"content": [{
        "startRef": "m00001", "endRef": "m00002", "summary": "intro",
        "topic": "setup", "summaryMaxChars": 5000
    }]}));
    let diagnostics = &parsed.diagnostics;
    assert_eq!(diagnostics.kind, ParseKind::Ok);
    assert!(diagnostics.ok);
    assert_eq!(diagnostics.invalid_items, 0);
    assert_eq!(refs(&parsed), [("m00001", "m00002")]);
    assert_eq!(parsed.ranges[0].summary, "intro");
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("setup"));
    assert_eq!(parsed.ranges[0].summary_max_chars, Some(5000));
}

#[test]
fn parses_a_raw_json_document_string() {
    let input =
        json!({"content": [{"startRef": "m00001", "endRef": "m00002", "summary": "intro"}]})
            .to_string();
    let parsed = parse_str(&input);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "intro");
    assert_eq!(parsed.diagnostics.length, Some(input.len()));
    assert_eq!(
        parsed.diagnostics.raw_prefix.as_deref(),
        Some(input.as_str())
    );
}

#[test]
fn unwraps_one_level_of_double_stringification() {
    let inner =
        json!({"content": [{"startRef": "m00001", "endRef": "m00002", "summary": "doubled"}]})
            .to_string();
    let parsed = parse_str(&Value::String(inner).to_string());
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "doubled");
}

#[test]
fn accepts_a_stringified_content_array() {
    let content =
        json!([{"startRef": "m00001", "endRef": "m00002", "summary": "vllm"}]).to_string();
    let parsed = parse(json!({ "content": content }));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(refs(&parsed), [("m00001", "m00002")]);
    assert_eq!(parsed.ranges[0].summary, "vllm");
}

fn two_entries() -> Value {
    json!([
        {"startRef": "m00001", "endRef": "m00002", "summary": "first entry complete"},
        {"startRef": "m00003", "endRef": "m00004", "summary": "second entry cut off mid"}
    ])
}

#[test]
fn salvages_a_stringified_content_array_truncated_mid_entry() {
    let full = two_entries().to_string();
    let parsed = parse(json!({ "content": &full[..full.len() - 12] }));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
    assert!(parsed.diagnostics.ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "first entry complete");
    assert_eq!(parsed.diagnostics.invalid_items, 0);
}

#[test]
fn strips_markdown_fences_with_and_without_a_language_tag() {
    let doc = json!({"content": [{"startRef": "m00001", "endRef": "m00002", "summary": "fenced"}]})
        .to_string();
    for input in [format!("```json\n{doc}\n```"), format!("```\n{doc}\n```")] {
        let parsed = parse_str(&input);
        assert_eq!(parsed.diagnostics.kind, ParseKind::Ok, "{input}");
        assert_eq!(parsed.ranges.len(), 1);
        assert_eq!(parsed.ranges[0].summary, "fenced");
    }
}

#[test]
fn repairs_trailing_commas() {
    let parsed =
        parse_str(r#"{"content": [{"startRef": "m00001", "endRef": "m00002", "summary": "S",}]}"#);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "S");
}

#[test]
fn escapes_raw_newlines_inside_string_values() {
    let parsed = parse_str(
        "{\"content\": [{\"startRef\": \"m00001\", \"endRef\": \"m00002\", \"summary\": \"line1\nline2\"}]}",
    );
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "line1\nline2");
}

#[test]
fn preserves_escaped_quotes_in_salvaged_entries() {
    let doc = json!({"content": [
        {"startRef": "m00001", "endRef": "m00002", "summary": "has \"quotes\" inside"},
        {"startRef": "m00003", "endRef": "m00004", "summary": "gone"}
    ]})
    .to_string();
    let first_close = doc.find('}').unwrap() + 1;
    let input = format!(
        "{}, {{\"startRef\": \"m00003\", \"endRef\": \"m0",
        &doc[..first_close]
    );
    let parsed = parse_str(&input);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "has \"quotes\" inside");
}

// Single-quoted JSON salvage (#603).

#[test]
fn salvages_single_quoted_json_args() {
    let input = "{'content':[{'startId':'m00010','endId':'m00020','summary':'first'},{'startId':'m00030','endId':'m00040','summary':'second','topic':'mid'}],'topic':'intro'}";
    let parsed = parse_compress_args(&Value::String(input.to_owned()), Some("call-q"));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.quote_salvage);
    assert_eq!(refs(&parsed), [("m00010", "m00020"), ("m00030", "m00040")]);
    assert_eq!(parsed.ranges[0].summary, "first");
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("intro"));
    assert_eq!(parsed.ranges[0].compress_call_id.as_deref(), Some("call-q"));
    assert_eq!(parsed.ranges[1].topic.as_deref(), Some("mid"));
}

#[test]
fn salvages_mixed_quotes_keeping_apostrophes_in_data() {
    let parsed = parse_str(
        r#"{'content': [{'startId': "m00001", 'endId': "m00009", 'summary': "it's done, really"}]}"#,
    );
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.quote_salvage);
    assert_eq!(refs(&parsed), [("m00001", "m00009")]);
    assert_eq!(parsed.ranges[0].summary, "it's done, really");
}

#[test]
fn salvages_a_truncated_single_quoted_content_array() {
    let parsed = parse_str("{'content':[{'startId':'m00010','endId':'m00020','summary':'first'},{'startId':'m00030','endId':'m00040','summary':'secon");
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
    assert!(parsed.diagnostics.ok);
    assert!(parsed.diagnostics.quote_salvage);
    assert_eq!(refs(&parsed), [("m00010", "m00020")]);
    assert_eq!(parsed.ranges[0].summary, "first");
}

#[test]
fn salvages_an_object_whose_content_string_is_single_quoted() {
    let parsed =
        parse(json!({"content": "[{'startId':'m00005','endId':'m00006','summary':'ok'}]"}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.quote_salvage);
    assert_eq!(refs(&parsed), [("m00005", "m00006")]);
    assert_eq!(parsed.ranges[0].summary, "ok");
}

#[test]
fn rejects_prose_with_apostrophes_unchanged() {
    let parsed = parse_str("I couldn't believe {that} would 'work'");
    assert!(parsed.ranges.is_empty());
    assert!(!parsed.diagnostics.ok);
    assert_eq!(parsed.diagnostics.kind, ParseKind::MalformedJson);
    assert!(!parsed.diagnostics.quote_salvage);
}

#[test]
fn leaves_valid_double_quoted_args_untouched_by_the_salvage() {
    let input =
        json!({"content": [{"startId": "m00001", "endId": "m00002", "summary": "don't panic"}]})
            .to_string();
    let parsed = parse_str(&input);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(!parsed.diagnostics.quote_salvage);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "don't panic");
}

#[test]
fn repairs_single_quotes_with_trailing_commas_and_raw_newlines() {
    let parsed =
        parse_str("{'content':[{'startId':'m00001','endId':'m00002','summary':'line1\nline2',}]}");
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.quote_salvage);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "line1\nline2");
}

#[test]
fn does_not_fabricate_fields_for_a_single_quoted_entry_missing_bounds() {
    let parsed = parse_str("{'content':[{'startId':'m00010'}]}");
    assert!(parsed.ranges.is_empty());
    assert!(!parsed.diagnostics.ok);
    assert!(!parsed.diagnostics.quote_salvage);
}

// Truncation salvage.

#[test]
fn salvages_complete_entries_from_a_truncated_prefix() {
    let doc = json!({ "content": two_entries() }).to_string();
    let parsed = parse_str(&doc[..doc.len() - 12]);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
    assert!(parsed.diagnostics.ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "first entry complete");
}

#[test]
fn salvages_entries_when_the_closing_brackets_are_missing() {
    let entry = r#"{"startRef": "m00001", "endRef": "m00002", "summary": "no brackets after me"}"#;
    let parsed = parse_str(&format!(
        "{{\"content\": [{entry}, {{\"startRef\": \"m00003\""
    ));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
    assert!(parsed.diagnostics.ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "no brackets after me");
}

#[test]
fn reports_truncated_without_ranges_when_the_first_entry_is_cut() {
    let parsed = parse_str(r#"{"content": [{"startRef": "m00001", "endRef": "m0"#);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
    assert!(!parsed.diagnostics.ok);
    assert!(parsed.ranges.is_empty());
}

#[test]
fn does_not_fabricate_partial_entries() {
    let parsed = parse_str(r#"{"content": [{"startRef": "m00001", "summary": "par"#);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
    assert!(parsed.ranges.is_empty());
}

// Error classification.

#[test]
fn classifies_empty_inputs() {
    for input in [Value::Null, json!(""), json!("   ")] {
        let parsed = parse(input.clone());
        assert_eq!(parsed.diagnostics.kind, ParseKind::EmptyInput, "{input}");
        assert!(!parsed.diagnostics.ok);
        assert!(parsed.ranges.is_empty());
    }
}

#[test]
fn classifies_non_object_parsed_values() {
    for input in [
        json!([]),
        json!(42),
        json!("\"just a string\""),
        json!("true"),
    ] {
        let parsed = parse(input.clone());
        assert_eq!(parsed.diagnostics.kind, ParseKind::NotObject, "{input}");
        assert!(!parsed.diagnostics.ok);
        assert!(parsed.ranges.is_empty());
    }
}

#[test]
fn classifies_an_object_without_a_content_key() {
    let parsed = parse(json!({"foo": 1, "bar": "x"}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::MissingContent);
    assert!(!parsed.diagnostics.ok);
    // The kernel lists the keys in insertion order (`foo`, `bar`); a
    // `serde_json` object keeps them sorted.
    let mut keys = parsed.diagnostics.keys.clone();
    keys.sort();
    assert_eq!(keys, ["bar", "foo"]);
}

#[test]
fn classifies_content_that_is_neither_an_array_nor_an_array_string() {
    let parsed = parse(json!({"content": "123"}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::ContentNotArray);
    assert!(!parsed.diagnostics.ok);
}

#[test]
fn classifies_balanced_garbage_as_malformed_json() {
    let parsed = parse_str("hello");
    assert_eq!(parsed.diagnostics.kind, ParseKind::MalformedJson);
    assert!(!parsed.diagnostics.ok);
    assert!(parsed.ranges.is_empty());
}

#[test]
fn counts_invalid_entries_and_keeps_the_valid_ones() {
    let parsed = parse(json!({"content": [
        {"startRef": "m00001", "endRef": "m00002", "summary": "good"},
        {"summary": "missing refs"},
        {"startRef": "m00003", "endRef": "m00004"},
        {"startRef": "m00005", "endRef": "m00006", "summary": 42},
        "garbage",
        null
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "good");
    assert_eq!(parsed.diagnostics.invalid_items, 5);
}

#[test]
fn reports_per_entry_invalid_reasons() {
    let parsed = parse(json!({"content": [
        {"startRef": "m00001", "endRef": "m00002", "summary": "good"},
        {"summary": "missing refs"},
        {"startRef": "m00003", "endRef": "m00004"},
        "garbage"
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.diagnostics.invalid_items, 3);
    assert_eq!(
        parsed.diagnostics.invalid_reasons,
        [
            "entry 1: missing range bounds (need startRef/startId and endRef/endId)",
            "entry 2: missing summary",
            "entry 3: line entry: no mNNNNN/bN refs in header",
        ]
    );
}

#[test]
fn reports_invalid_reasons_for_no_valid_ranges() {
    let parsed = parse(json!({"content": [
        {"startRef": "m00001", "endRef": "m00002"},
        {"summary": "no bounds"}
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::NoValidRanges);
    assert_eq!(parsed.diagnostics.invalid_items, 2);
    assert_eq!(
        parsed.diagnostics.invalid_reasons,
        [
            "entry 0: missing summary",
            "entry 1: missing range bounds (need startRef/startId and endRef/endId)",
        ]
    );
}

#[test]
fn reports_no_valid_ranges_for_an_empty_content_array_or_string() {
    let parsed = parse(json!({"content": []}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::NoValidRanges);
    assert!(!parsed.diagnostics.ok);
    assert!(parsed.ranges.is_empty());
    assert_eq!(parsed.diagnostics.invalid_items, 0);
    let parsed = parse(json!({"content": ""}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::NoValidRanges);
    assert!(parsed.ranges.is_empty());
}

// Field-name variants and call-id stamping.

#[test]
fn accepts_start_id_end_id_and_message_id_variants() {
    let parsed = parse(json!({"content": [
        {"startId": "m00001", "endId": "m00002", "summary": "variant-a"},
        {"messageId": "m00003", "summary": "variant-b"}
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(refs(&parsed), [("m00001", "m00002"), ("m00003", "m00003")]);
}

#[test]
fn stamps_the_compress_call_id_when_one_is_given() {
    let input =
        json!({"content": [{"startRef": "m00001", "endRef": "m00002", "summary": "stamped"}]});
    let stamped = parse_compress_args(&input, Some("call-9"));
    assert_eq!(
        stamped.ranges[0].compress_call_id.as_deref(),
        Some("call-9")
    );
    assert_eq!(parse(input).ranges[0].compress_call_id, None);
}

// Diagnostics shape.

#[test]
fn caps_the_raw_prefix_at_800_and_reports_the_full_length() {
    let parsed = parse_str(&"x".repeat(1000));
    assert_eq!(parsed.diagnostics.kind, ParseKind::MalformedJson);
    assert_eq!(parsed.diagnostics.length, Some(1000));
    assert_eq!(
        parsed.diagnostics.raw_prefix.as_deref().map(str::len),
        Some(800)
    );
}

#[test]
fn leaves_the_raw_prefix_and_length_unset_for_object_input() {
    let parsed = parse(json!({"content": []}));
    assert_eq!(parsed.diagnostics.raw_prefix, None);
    assert_eq!(parsed.diagnostics.length, None);
}

// Top-level shapes the adapters observe.

#[test]
fn accepts_a_single_range_at_the_top_level() {
    let parsed = parse(json!({"startRef": "m00001", "endRef": "m00002", "summary": "single"}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(refs(&parsed), [("m00001", "m00002")]);
    assert_eq!(parsed.ranges[0].summary, "single");
    let variants = parse(json!({"startId": "m00001", "endId": "m00002", "summary": "single"}));
    assert_eq!(variants.diagnostics.kind, ParseKind::Ok);
    assert_eq!(variants.ranges[0].start_ref, "m00001");
}

#[test]
fn reports_missing_content_without_content_or_a_single_range() {
    let parsed = parse(json!({"foo": "bar"}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::MissingContent);
    assert!(parsed.ranges.is_empty());
}

#[test]
fn applies_a_top_level_topic_and_summary_cap_to_ranges_without_their_own() {
    let parsed = parse(json!({"topic": "Top", "summaryMaxChars": 5000, "content": [
        {"startRef": "m00001", "endRef": "m00002", "summary": "a"},
        {"startRef": "m00003", "endRef": "m00004", "summary": "b", "topic": "Own", "summaryMaxChars": 999}
    ]}));
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("Top"));
    assert_eq!(parsed.ranges[1].topic.as_deref(), Some("Own"));
    assert_eq!(parsed.ranges[0].summary_max_chars, Some(5000));
    assert_eq!(parsed.ranges[1].summary_max_chars, Some(999));
}

// Repairs of control characters and invalid escapes.

#[test]
fn repairs_raw_newlines_and_invalid_escapes_in_one_stringified_payload() {
    let summary = "m00066-m00081 Investigation reads: exact pre-edit code\n\n## Files to modify (`src/x.ts`)\nand some \\*markdown\\* with a \\`tick\\`";
    let content = format!(
        "[{{\"startRef\": \"m00066\", \"endRef\": \"m00081\", \"summary\": \"{summary}\"}}]"
    );
    let parsed = parse(json!({ "content": content }));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.ok);
    assert_eq!(refs(&parsed), [("m00066", "m00081")]);
    assert_eq!(parsed.ranges[0].summary, summary);
}

fn document_with_summary(summary: &str) -> String {
    format!("{{\"content\": [{{\"startRef\": \"m00001\", \"endRef\": \"m00002\", \"summary\": \"{summary}\"}}]}}")
}

#[test]
fn repairs_scattered_control_characters_in_one_pass() {
    let summary = "line one\ttabbed\rreturn\nnewline then a \\*star\\*";
    let parsed = parse_str(&document_with_summary(summary));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, summary);
}

#[test]
fn keeps_an_invalid_escapes_backslash_literally() {
    let parsed = parse_str(&document_with_summary("a \\* b"));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges[0].summary, "a \\* b");
}

#[test]
fn leaves_valid_json_and_structural_garbage_untouched_by_the_repair() {
    let valid =
        json!({"content": [{"startRef": "m00001", "endRef": "m00002", "summary": "clean"}]});
    assert_eq!(
        parse_str(&valid.to_string()).diagnostics.kind,
        ParseKind::Ok
    );
    let garbage = parse_str(r#"{"content": [{"startRef": "m00001", "summary": "unterminated"#);
    assert!(matches!(
        garbage.diagnostics.kind,
        ParseKind::Truncated | ParseKind::MalformedJson
    ));
    assert!(garbage.ranges.is_empty());
}

#[test]
fn maps_other_control_characters_through_unicode_escapes() {
    let summary = "a\u{1}b";
    let parsed = parse_str(&document_with_summary(summary));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges[0].summary, summary);
}

// One entry object where the content array belongs (#470).

#[test]
fn wraps_a_single_entry_object_in_the_content_slot() {
    let parsed = parse(
        json!({"content": {"startId": "m04083", "endId": "m04092", "summary": "lost range retry"}}),
    );
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.ok);
    assert!(parsed.diagnostics.content_salvage);
    assert_eq!(parsed.diagnostics.invalid_items, 0);
    assert_eq!(refs(&parsed), [("m04083", "m04092")]);
    assert_eq!(parsed.ranges[0].summary, "lost range retry");
}

#[test]
fn a_wrapped_entry_keeps_its_fields_and_takes_the_top_level_ones() {
    let own = parse(json!({"content": {
        "startRef": "m00001", "endRef": "m00002", "summary": "s", "topic": "Own", "summaryMaxChars": 1234
    }}));
    assert_eq!(own.diagnostics.kind, ParseKind::Ok);
    assert_eq!(own.ranges[0].topic.as_deref(), Some("Own"));
    assert_eq!(own.ranges[0].summary_max_chars, Some(1234));
    let top = parse(
        json!({"topic": "Top", "content": {"startRef": "m00001", "endRef": "m00002", "summary": "s"}}),
    );
    assert_eq!(top.ranges[0].topic.as_deref(), Some("Top"));
    let stamped = parse_compress_args(
        &json!({"content": {"startRef": "m00001", "endRef": "m00002", "summary": "s"}}),
        Some("call-470"),
    );
    assert_eq!(
        stamped.ranges[0].compress_call_id.as_deref(),
        Some("call-470")
    );
}

#[test]
fn recovers_string_encoded_single_object_content() {
    let input = json!({"content": {"startId": "m04083", "endId": "m04092", "summary": "str"}});
    let parsed = parse_str(&input.to_string());
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.content_salvage);
    assert_eq!(refs(&parsed), [("m04083", "m04092")]);
    assert_eq!(parsed.ranges[0].summary, "str");
}

#[test]
fn recovers_fenced_double_stringified_single_object_content() {
    let inner =
        json!({"content": {"startId": "m00001", "endId": "m00002", "summary": "solo"}}).to_string();
    let parsed = parse_str(&format!("```json\n{}\n```", Value::String(inner)));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.content_salvage);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "solo");
}

#[test]
fn unwraps_a_nested_ranges_array_in_the_content_slot() {
    let parsed = parse(json!({"content": {"ranges": [
        {"startRef": "m00001", "endRef": "m00002", "summary": "a"},
        {"startRef": "m00003", "endRef": "m00004", "summary": "b"}
    ]}}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(parsed.diagnostics.content_salvage);
    assert_eq!(parsed.ranges.len(), 2);
    assert_eq!(parsed.ranges[0].summary, "a");
    assert_eq!(parsed.ranges[1].summary, "b");
}

#[test]
fn a_wrapped_garbage_object_reports_per_entry_reasons() {
    let parsed = parse(json!({"content": {"foo": 1}}));
    assert!(parsed.ranges.is_empty());
    assert!(!parsed.diagnostics.ok);
    assert_eq!(parsed.diagnostics.kind, ParseKind::NoValidRanges);
    assert!(parsed.diagnostics.content_salvage);
    assert_eq!(parsed.diagnostics.invalid_items, 1);
    assert!(first_reason(&parsed).contains("missing range bounds"));
    let incomplete = parse(json!({"content": {"startId": "m00001", "endId": "m00002"}}));
    assert!(incomplete.ranges.is_empty());
    assert_eq!(incomplete.diagnostics.kind, ParseKind::NoValidRanges);
    assert_eq!(incomplete.diagnostics.invalid_items, 1);
    assert!(first_reason(&incomplete).contains("missing summary"));
}

#[test]
fn drops_a_truncated_single_object_content_entry_without_guessing() {
    let parsed = parse_str(r#"{"content": {"startId": "m00001", "endId": "m0"#);
    assert!(parsed.ranges.is_empty());
    assert!(!parsed.diagnostics.ok);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Truncated);
}

#[test]
fn keeps_content_not_array_for_non_object_content_values() {
    for input in [
        json!({"content": 42}),
        json!({"content": null}),
        json!({"content": true}),
    ] {
        let parsed = parse(input.clone());
        assert_eq!(
            parsed.diagnostics.kind,
            ParseKind::ContentNotArray,
            "{input}"
        );
        assert!(!parsed.diagnostics.ok);
        assert!(!parsed.diagnostics.content_salvage, "{input}");
        assert!(parsed.ranges.is_empty());
    }
}

#[test]
fn leaves_canonical_array_input_unmarked_by_content_salvage() {
    let parsed =
        parse(json!({"content": [{"startRef": "m00001", "endRef": "m00002", "summary": "clean"}]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert!(!parsed.diagnostics.content_salvage);
    assert_eq!(parsed.ranges.len(), 1);
}

// Flat single range: the summary in the `content` string, the refs at the
// top level.

#[test]
fn recovers_a_flat_single_range_with_its_summary_in_content() {
    let parsed = parse(json!({
        "content": "Summary of the folded range.\nSecond line of the summary.",
        "startId": "m00150",
        "endId": "m00168"
    }));
    assert!(parsed.diagnostics.ok);
    assert!(parsed.diagnostics.flat_summary_salvage);
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(refs(&parsed), [("m00150", "m00168")]);
    assert_eq!(
        parsed.ranges[0].summary,
        "Summary of the folded range.\nSecond line of the summary."
    );
}

#[test]
fn a_flat_single_range_keeps_the_top_level_topic_and_ref_variants() {
    let parsed = parse(json!({
        "content": "The summary text.", "startRef": "m00002", "endRef": "m00009", "topic": "repro loop"
    }));
    assert_eq!(refs(&parsed), [("m00002", "m00009")]);
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("repro loop"));
}

#[test]
fn a_flat_single_range_unwraps_a_json_summary_and_topic_in_content() {
    let parsed = parse(json!({
        "content": json!({"summary": "encoded summary", "topic": "inner topic"}).to_string(),
        "startId": "m00001",
        "endId": "m00004"
    }));
    assert!(parsed.diagnostics.flat_summary_salvage);
    assert_eq!(parsed.ranges[0].summary, "encoded summary");
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("inner topic"));
}

#[test]
fn a_flat_single_range_prefers_an_explicit_top_level_summary() {
    let parsed = parse(json!({
        "content": "ignored prose", "summary": "explicit summary wins", "startId": "m00001", "endId": "m00002"
    }));
    assert_eq!(parsed.ranges[0].summary, "explicit summary wins");
}

#[test]
fn no_flat_single_range_when_content_carries_refs_headers() {
    let ok = parse(
        json!({"content": "m00010–m00020 topic\nsummary text.", "startId": "m00001", "endId": "m00099"}),
    );
    assert_eq!(refs(&ok), [("m00010", "m00020")]);
    assert!(!ok.diagnostics.flat_summary_salvage);
    let bad = parse(
        json!({"content": "m00010–m00020 topic only", "startId": "m00001", "endId": "m00099"}),
    );
    assert!(!bad.diagnostics.ok);
    assert!(!bad.diagnostics.flat_summary_salvage);
    assert_eq!(bad.diagnostics.kind, ParseKind::NoValidRanges);
    assert!(first_reason(&bad).contains("missing summary"));
}

#[test]
fn headerless_prose_without_top_level_refs_stays_content_not_array() {
    let parsed = parse(json!({"content": "just prose, no refs, no bounds"}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::ContentNotArray);
    assert!(!parsed.diagnostics.flat_summary_salvage);
}

// The relocation pointer echoed as a new call, and one stringified entry.

#[test]
fn content_echoing_the_relocation_pointer_is_an_already_folded_echo() {
    let wrapped = parse(json!({
        "content": "{\"content\": \"[bili] compression succeeded — summary relocated to its fold point above (block(s) b49); these ranges are already folded — do not re-compress them.\"}"
    }));
    assert!(!wrapped.diagnostics.ok);
    assert_eq!(wrapped.diagnostics.kind, ParseKind::AlreadyFoldedEcho);
    assert!(wrapped.ranges.is_empty());
    let bare = parse(json!({
        "content": "[bili] compression succeeded — summary relocated to its fold point above (block(s) b50); these ranges are already folded — do not re-compress them."
    }));
    assert_eq!(bare.diagnostics.kind, ParseKind::AlreadyFoldedEcho);
}

#[test]
fn a_content_string_encoding_one_entry_object_is_salvaged() {
    let entry = json!({"startId": "m00150", "endId": "m00220", "summary": "investigated the loop; fixed the gate"});
    let parsed = parse(json!({ "content": entry.to_string() }));
    assert!(parsed.diagnostics.ok);
    assert!(parsed.diagnostics.content_salvage);
    assert_eq!(refs(&parsed), [("m00150", "m00220")]);
    assert_eq!(
        parsed.ranges[0].summary,
        "investigated the loop; fixed the gate"
    );
}

#[test]
fn a_stringified_object_without_range_bounds_still_rejects() {
    let parsed = parse(json!({ "content": json!({"note": "not a range"}).to_string() }));
    assert!(!parsed.diagnostics.ok);
    assert_eq!(parsed.diagnostics.kind, ParseKind::ContentNotArray);
}
