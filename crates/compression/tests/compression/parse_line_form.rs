// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the line-form cases of `kernel/tests/parse-compress-input.test.ts`.

use hanafubuki_agent_compression::{derive_topic_from_summary, ParseKind};
use serde_json::json;

use crate::parse::{first_reason, parse, parse_str, refs};

// Line-form entries.

#[test]
fn line_form_array_of_strings_keeps_the_markdown_summary_verbatim() {
    let parsed = parse(json!({"content": [
        "m00150–m00220 debug part 1\n## Fixed the loop\nbody with | pipes | and \"quotes\" and \\backslashes\\",
        "m00300-m00350\nplain summary without heading"
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(refs(&parsed), [("m00150", "m00220"), ("m00300", "m00350")]);
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("debug part 1"));
    assert_eq!(
        parsed.ranges[0].summary,
        "## Fixed the loop\nbody with | pipes | and \"quotes\" and \\backslashes\\"
    );
    assert_eq!(
        parsed.ranges[1].topic.as_deref(),
        Some("plain summary without heading")
    );
}

#[test]
fn line_form_topic_is_the_first_heading_else_the_first_line_cut_to_60() {
    let parsed = parse(json!({"content": [
        "m00010–m00020\n## The Heading Wins\ndetail",
        "m00030–m00040\nfirst plain line is the topic\nmore",
        format!("m00050–m00060\n{}\ntail", "x".repeat(80))
    ]}));
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("The Heading Wins"));
    assert_eq!(
        parsed.ranges[1].topic.as_deref(),
        Some("first plain line is the topic")
    );
    assert_eq!(parsed.ranges[2].topic.as_deref().map(str::len), Some(60));
}

#[test]
fn derive_topic_strips_a_bullet_and_ignores_blank_text() {
    assert_eq!(
        derive_topic_from_summary("- first bullet\nmore").as_deref(),
        Some("first bullet")
    );
    assert_eq!(
        derive_topic_from_summary("\n\n### Deep heading\n").as_deref(),
        Some("Deep heading")
    );
    assert_eq!(derive_topic_from_summary("  \n "), None);
}

#[test]
fn line_form_mixes_with_object_entries_validated_independently() {
    let parsed = parse(json!({"content": [
        "m00001–m00002\nstring entry summary",
        {"startId": "m00003", "endId": "m00004", "summary": "object entry", "topic": "Obj"},
        "no refs here at all\nsummary",
        {"startId": "m00005"}
    ]}));
    assert_eq!(parsed.ranges.len(), 2);
    assert_eq!(parsed.diagnostics.invalid_items, 2);
    assert_eq!(parsed.ranges[0].summary, "string entry summary");
    assert_eq!(parsed.ranges[1].topic.as_deref(), Some("Obj"));
}

#[test]
fn line_form_single_entry_and_single_bare_ref_entry() {
    let pair = parse(json!({"content": ["m00007—m00009\nsummary here"]}));
    assert_eq!(refs(&pair), [("m00007", "m00009")]);
    let single = parse(json!({"content": ["m00042 lone ref line\nsummary"]}));
    assert_eq!(refs(&single), [("m00042", "m00042")]);
}

#[test]
fn line_form_accepts_every_separator() {
    for header in [
        "m00150~m00220",
        "m00150 to m00220",
        "m00150…m00220",
        "m00150...m00220",
        "m00150..m00220",
        "m00150−m00220",
        "m00150－m00220",
    ] {
        let parsed = parse(json!({ "content": [format!("{header}\nsum")] }));
        assert_eq!(parsed.diagnostics.kind, ParseKind::Ok, "{header}");
        assert_eq!(refs(&parsed), [("m00150", "m00220")], "{header}");
    }
    for header in [
        "m00150−m00168",
        "m00150－m00168",
        "m00150..m00168",
        "m00150...m00168",
    ] {
        let parsed = parse(json!({ "content": format!("{header} topic\nsummary.") }));
        assert_eq!(refs(&parsed), [("m00150", "m00168")], "{header}");
    }
}

#[test]
fn line_form_header_without_a_summary_is_dropped_and_the_others_survive() {
    let parsed = parse(json!({"content": ["m00001–m00005 topic\nfirst summary", "m00010–m00012"]}));
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "first summary");
    assert_eq!(parsed.diagnostics.invalid_items, 1);
    assert!(first_reason(&parsed).contains("missing summary"));
}

#[test]
fn line_form_bare_string_content_splits_on_ref_pair_headers() {
    let parsed = parse(json!({
        "content": "m00100–m00110 part A\n## A\nsummary A\nm00200–m00220 part B\nsummary B"
    }));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 2);
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("part A"));
    assert_eq!(parsed.ranges[0].summary, "## A\nsummary A");
    assert_eq!(parsed.ranges[1].summary, "summary B");
}

#[test]
fn line_form_stringified_array_of_strings_goes_through_the_string_path() {
    let input = json!({"content": ["m00111–m00122\nsum one", "m00333–m00344 t2\nsum two"]});
    let parsed = parse_str(&input.to_string());
    assert_eq!(parsed.ranges.len(), 2);
    assert_eq!(parsed.ranges[0].summary, "sum one");
    assert_eq!(parsed.ranges[1].topic.as_deref(), Some("t2"));
}

#[test]
fn line_form_empty_string_entry_is_invalid_not_a_crash() {
    let parsed = parse(json!({"content": ["", "m00001–m00002\nok"]}));
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.diagnostics.invalid_items, 1);
}

#[test]
fn line_form_block_refs_and_zero_padded_message_refs() {
    let blocks = parse(json!({"content": [
        "b3–b15 tier fold\n## Folded\nsummary of blocks",
        "b2 lone block\nsummary"
    ]}));
    assert_eq!(refs(&blocks), [("b3", "b15"), ("b2", "b2")]);
    let padded = parse(json!({"content": ["m150–m220 pad\nsummary"]}));
    assert_eq!(refs(&padded), [("m00150", "m00220")]);
}

#[test]
fn line_form_refs_cited_in_the_body_never_become_the_range() {
    let parsed = parse(json!({"content": [
        "m00042 lone ref line\nEarlier I compressed m00300–m00400 for the auth work.\nDetails here."
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(refs(&parsed), [("m00042", "m00042")]);
    assert_eq!(
        parsed.ranges[0].summary,
        "Earlier I compressed m00300–m00400 for the auth work.\nDetails here."
    );
}

#[test]
fn line_form_entry_without_refs_on_its_first_line_is_dropped() {
    let parsed = parse(json!({"content": [
        "Summary of work\nm00150–m00220 were discussed before\nmore detail",
        "m00001–m00002 ok entry\nfine"
    ]}));
    assert_eq!(refs(&parsed), [("m00001", "m00002")]);
    assert_eq!(parsed.diagnostics.invalid_items, 1);
    assert!(first_reason(&parsed).contains("no mNNNNN/bN refs in header"));
}

#[test]
fn line_form_header_matches_on_the_first_line_only() {
    let parsed = parse(json!({"content": [
        "## Auth exploration\nFound token in m00042. Decisions made:\n- chose X because Y",
        "Explored the range m00150–m00220 earlier.\nSummary body here",
        "m00007–m00009 real header\ncites m00065 and m00150–m00220 in the body"
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.diagnostics.invalid_items, 2);
    assert!(parsed
        .diagnostics
        .invalid_reasons
        .iter()
        .all(|reason| reason.contains("no mNNNNN/bN refs in header")));
    assert_eq!(refs(&parsed), [("m00007", "m00009")]);
    assert_eq!(
        parsed.ranges[0].summary,
        "cites m00065 and m00150–m00220 in the body"
    );
}

#[test]
fn line_form_crlf_header_trims_the_carriage_return_from_the_topic() {
    let parsed = parse(json!({"content": ["m00001–m00005 crlf topic\r\nbody line"]}));
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("crlf topic"));
    assert_eq!(parsed.ranges[0].summary, "body line");
}

// Split line-form entries: header and summary as sibling strings (#1001).

#[test]
fn coalesces_a_header_with_its_summary_sibling() {
    let parsed = parse(json!({"content": [
        "m01588\u{2013}m01712 走bili UI move + proxy 调试",
        "## TASK AS OF THIS BLOCK\n- user goal\n- decisions with reasons"
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.diagnostics.invalid_items, 0);
    assert_eq!(refs(&parsed), [("m01588", "m01712")]);
    assert_eq!(
        parsed.ranges[0].summary,
        "## TASK AS OF THIS BLOCK\n- user goal\n- decisions with reasons"
    );
    assert_eq!(
        parsed.ranges[0].topic.as_deref(),
        Some("走bili UI move + proxy 调试")
    );
}

#[test]
fn coalesces_several_summary_siblings_onto_one_header() {
    let parsed =
        parse(json!({"content": ["m00150\u{2013}m00220", "first summary chunk", "second chunk"]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(
        parsed.ranges[0].summary,
        "first summary chunk\nsecond chunk"
    );
}

#[test]
fn keeps_a_stray_refless_string_after_a_complete_entry_invalid() {
    let parsed = parse(json!({"content": [
        "m00001\u{2013}m00002 topic\nreal summary",
        "## stray with no pending header"
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "real summary");
    assert_eq!(parsed.diagnostics.invalid_items, 1);
}

#[test]
fn rejects_a_header_only_entry_with_nothing_to_adopt() {
    let parsed = parse(json!({"content": ["m00001\u{2013}m00002 topic"]}));
    assert!(parsed.ranges.is_empty());
    assert_eq!(parsed.diagnostics.kind, ParseKind::NoValidRanges);
    assert!(first_reason(&parsed).contains("missing summary"));
}

#[test]
fn an_object_entry_resets_the_adoption() {
    let parsed = parse(json!({"content": [
        {"startRef": "m00001", "endRef": "m00002", "summary": "good"},
        "orphan summary body"
    ]}));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.ranges.len(), 1);
    assert_eq!(parsed.ranges[0].summary, "good");
    assert_eq!(parsed.diagnostics.invalid_items, 1);
}

#[test]
fn one_bare_string_carries_a_whole_batch() {
    let parsed = parse(json!({
        "content": "m00001–m00005 topic one\nsummary one line.\nm00006–m00008\ntopic-less summary."
    }));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    assert_eq!(parsed.diagnostics.invalid_items, 0);
    assert_eq!(refs(&parsed), [("m00001", "m00005"), ("m00006", "m00008")]);
    assert_eq!(parsed.ranges[0].topic.as_deref(), Some("topic one"));
    assert_eq!(parsed.ranges[0].summary, "summary one line.");
    assert_eq!(parsed.ranges[1].summary, "topic-less summary.");
}

// The line-form split is linear (a ReDoS regression).

#[test]
fn line_form_split_stays_linear_on_a_long_whitespace_run() {
    for content in [
        format!("x{}x", "\n".repeat(50_000)),
        format!("x\n\",{}x", " ".repeat(50_000)),
        format!("x{}y]", " ".repeat(50_000)),
    ] {
        let started = std::time::Instant::now();
        let parsed = parse(json!({ "content": content }));
        assert_eq!(parsed.diagnostics.kind, ParseKind::ContentNotArray);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}

#[test]
fn line_form_split_breaks_on_headers_after_blank_lines_and_element_residue() {
    let parsed = parse(json!({
        "content": "m00001-m00005 first\nsummary one.\n\n  \"m00006-m00008 second\nsummary two.\"\n\t\n\"m00009-m00010\nsummary three."
    }));
    assert_eq!(parsed.diagnostics.kind, ParseKind::Ok);
    let triples: Vec<(&str, &str, &str)> = parsed
        .ranges
        .iter()
        .map(|range| {
            (
                range.start_ref.as_str(),
                range.end_ref.as_str(),
                range.summary.as_str(),
            )
        })
        .collect();
    assert_eq!(
        triples,
        [
            ("m00001", "m00005", "summary one."),
            ("m00006", "m00008", "summary two."),
            ("m00009", "m00010", "summary three."),
        ]
    );
}

#[test]
fn line_form_entries_split_at_two_dot_headers() {
    let parsed =
        parse(json!({"content": "m00001..m00003 first\nsum A\nm00004..m00007 second\nsum B"}));
    assert_eq!(refs(&parsed), [("m00001", "m00003"), ("m00004", "m00007")]);
}

#[test]
fn header_only_line_entries_report_no_valid_ranges_with_reasons() {
    let some =
        parse(json!({"content": "m00010–m00020 topic one\nsummary ok.\nm00030–m00040 topic two"}));
    assert_eq!(some.diagnostics.kind, ParseKind::Ok);
    assert_eq!(some.diagnostics.invalid_items, 1);
    assert!(first_reason(&some).contains("missing summary"));
    let none = parse(json!({"content": "m00010–m00020 topic only"}));
    assert_eq!(none.diagnostics.kind, ParseKind::NoValidRanges);
    assert!(first_reason(&none).contains("missing summary"));
}

// A pair divider the separators do not know never narrows a range.

#[test]
fn refuses_to_narrow_a_ref_pair_with_an_unrecognized_separator() {
    let parsed = parse(json!({"content": "m00150 => m00168 topic\nsummary."}));
    assert!(parsed.ranges.is_empty());
    assert_eq!(parsed.diagnostics.kind, ParseKind::NoValidRanges);
    assert!(first_reason(&parsed).contains("separator not recognized"));
    // A single-ref header with a punctuation topic still parses.
    let single = parse(json!({"content": "m00150 — intro notes\nsummary."}));
    assert_eq!(refs(&single), [("m00150", "m00150")]);
}
