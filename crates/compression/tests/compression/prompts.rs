// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `prompts.test.ts`, `compress-tools.test.ts` and
// `compress-schema-contract.test.ts`, and the byte-exact system-prompt
// fixture: `golden/prompt-function-default.txt` is a verbatim copy of
// `kernel/tests/fixtures/prompt-function-default.txt`.

use hanafubuki_agent_compression::{
    acp_tools, build_compress_system_prompt, compress_parameters, parse_compress_args,
    render_nudge_text, CompressibleRange, NudgeDecision, Prompts, ACP_CACHE_TOOL_DESCRIPTION,
    ACP_MUTATING_TOOLS, ACP_READONLY_TOOLS, ACP_TOOL_NAMES, COMPRESS_PHILOSOPHY,
    HOW_TO_COMPRESS_RULES, LANGUAGE_PRESERVATION_RULE, TIER2_DISTILL_RULES, TIER3_CONDENSE_RULES,
};
use serde_json::{json, Map, Value};

const PROMPT_FUNCTION_DEFAULT: &str = include_str!("golden/prompt-function-default.txt");

#[test]
fn the_default_system_prompt_is_byte_identical_to_the_kernel_fixture() {
    assert_eq!(
        build_compress_system_prompt(&Prompts::default(), false),
        PROMPT_FUNCTION_DEFAULT
    );
}

fn ranges(count: usize) -> Vec<CompressibleRange> {
    (0..count)
        .map(|index| CompressibleRange {
            start_ref: format!("m{:05}", index * 3 + 1),
            end_ref: format!("m{:05}", index * 3 + 3),
            count: 3,
            tokens: 1000 * (index as u64 + 1),
            ..CompressibleRange::default()
        })
        .collect()
}

fn decision(tier: Option<u8>, emergency: bool) -> NudgeDecision {
    let mut decision = NudgeDecision {
        should_inject: true,
        reason: "test".to_owned(),
        compressible_ranges: ranges(3),
        context_usage: if emergency { 0.99 } else { 0.5 },
        tier,
        ..NudgeDecision::default()
    };
    decision.breakdown.emergency_override = emergency;
    decision
}

fn every_voice() -> [NudgeDecision; 4] {
    [
        decision(None, false),
        decision(None, true),
        decision(Some(2), false),
        decision(Some(3), false),
    ]
}

#[test]
fn default_prompts_mirror_the_verbatim_rule_constants() {
    let prompts = Prompts::default();
    assert_eq!(prompts.compress_philosophy, COMPRESS_PHILOSOPHY);
    assert_eq!(prompts.how_to_compress_rules, HOW_TO_COMPRESS_RULES);
    assert_eq!(prompts.tier2_distill_rules, TIER2_DISTILL_RULES);
    assert_eq!(prompts.tier3_condense_rules, TIER3_CONDENSE_RULES);
}

#[test]
fn language_preservation_is_opt_in_and_off_by_default() {
    for rules in [
        COMPRESS_PHILOSOPHY,
        HOW_TO_COMPRESS_RULES,
        TIER2_DISTILL_RULES,
        TIER3_CONDENSE_RULES,
    ] {
        assert!(!rules.contains(LANGUAGE_PRESERVATION_RULE));
    }
    let prompts = Prompts::default();
    for text in [
        &prompts.compress_philosophy,
        &prompts.how_to_compress_rules,
        &prompts.tier2_distill_rules,
        &prompts.tier3_condense_rules,
    ] {
        assert!(!text.contains(LANGUAGE_PRESERVATION_RULE));
    }
}

#[test]
fn language_preservation_appends_the_rule_to_every_canonical_rule() {
    let prompts = Prompts::with_language_preservation();
    assert!(prompts
        .compress_philosophy
        .ends_with(&format!("\n- {LANGUAGE_PRESERVATION_RULE}")));
    for text in [
        &prompts.how_to_compress_rules,
        &prompts.tier2_distill_rules,
        &prompts.tier3_condense_rules,
    ] {
        assert!(text.ends_with(&format!("\n\n{LANGUAGE_PRESERVATION_RULE}")));
    }
}

#[test]
fn nudge_voices_carry_the_language_rule_only_when_opted_in() {
    let preserving = Prompts::with_language_preservation();
    for decision in every_voice() {
        assert!(render_nudge_text(&decision, &preserving, false)
            .text
            .contains(LANGUAGE_PRESERVATION_RULE));
        assert!(!render_nudge_text(&decision, &Prompts::default(), false)
            .text
            .contains(LANGUAGE_PRESERVATION_RULE));
    }
}

#[test]
fn the_default_nudge_carries_the_philosophy_and_the_rules() {
    let text = render_nudge_text(&decision(None, false), &Prompts::default(), false).text;
    assert!(text.contains("Compression Philosophy:"));
    assert!(text.contains("HOW TO COMPRESS"));
}

#[test]
fn the_default_nudge_embeds_the_full_rule_text_verbatim() {
    for emergency in [false, true] {
        let text = render_nudge_text(&decision(None, emergency), &Prompts::default(), false).text;
        assert!(text.contains(COMPRESS_PHILOSOPHY));
        assert!(text.contains(HOW_TO_COMPRESS_RULES));
    }
}

#[test]
fn the_tier_voices_carry_their_tier_rules() {
    let prompts = Prompts::default();
    assert!(
        render_nudge_text(&decision(Some(2), false), &prompts, false)
            .text
            .contains(TIER2_DISTILL_RULES)
    );
    assert!(
        render_nudge_text(&decision(Some(3), false), &prompts, false)
            .text
            .contains(TIER3_CONDENSE_RULES)
    );
}

// compress-tools.test.ts

#[test]
fn the_tool_sets_classify_every_tool_by_mutation() {
    let mut names = ACP_TOOL_NAMES.to_vec();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "acp_cache",
            "acp_status",
            "compress",
            "decompress",
            "search_context"
        ]
    );
    let mut mutating = ACP_MUTATING_TOOLS.to_vec();
    mutating.sort_unstable();
    assert_eq!(mutating, ["compress", "decompress"]);
    let mut readonly = ACP_READONLY_TOOLS.to_vec();
    readonly.sort_unstable();
    assert_eq!(readonly, ["acp_cache", "acp_status", "search_context"]);
    let offered: Vec<&str> = acp_tools().iter().map(|tool| tool.name).collect();
    assert_eq!(offered, ACP_TOOL_NAMES);
}

#[test]
fn compress_takes_one_string_with_no_alternatives() {
    let tools = acp_tools();
    let compress = &tools[0];
    assert_eq!(compress.name, "compress");
    assert_eq!(compress.parameters, compress_parameters());
    assert_eq!(
        compress.parameters["properties"]["content"]["type"],
        "string"
    );
    assert!(compress.parameters["properties"]["content"]
        .get("anyOf")
        .is_none());
}

#[test]
fn the_system_prompt_mentions_the_compress_tool() {
    let prompt = build_compress_system_prompt(&Prompts::default(), false);
    assert!(prompt.contains("compress"));
    assert!(prompt.len() > 1000);
}

#[test]
fn acp_cache_names_the_unattributed_bucket_and_is_read_only() {
    assert!(ACP_CACHE_TOOL_DESCRIPTION.contains("upstream-ttl-or-client-rewrite"));
    assert!(!ACP_CACHE_TOOL_DESCRIPTION.contains("TTL expiry"));
    assert!(ACP_CACHE_TOOL_DESCRIPTION.contains("Read-only"));
    assert!(!ACP_CACHE_TOOL_DESCRIPTION.contains("loop"));
}

// compress-schema-contract.test.ts: a minimal JSON Schema validator covering
// exactly what the compress schema uses, standing in for a pre-validating
// host. Whatever the parser accepts, the schema must not reject.

fn type_ok(kind: &str, value: &Value) -> bool {
    match kind {
        "array" => value.is_array(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        _ => false,
    }
}

fn validate(schema: &Value, value: &Value) -> bool {
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        if !type_ok(kind, value) {
            return false;
        }
    }
    if let (Some(items), Some(entries)) = (schema.get("items"), value.as_array()) {
        if !entries.iter().all(|entry| validate(items, entry)) {
            return false;
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            if !required
                .iter()
                .filter_map(Value::as_str)
                .all(|key| object.contains_key(key))
            {
                return false;
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (key, sub) in properties {
                if object.get(key).is_some_and(|value| !validate(sub, value)) {
                    return false;
                }
            }
        }
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        if !any_of.iter().any(|sub| validate(sub, value)) {
            return false;
        }
    }
    true
}

fn accepted_by_both(input: Value, expected: usize) {
    assert!(
        validate(&compress_parameters(), &input),
        "schema rejected a shape the parser accepts: {input}"
    );
    let parsed = parse_compress_args(&input, None);
    assert_eq!(parsed.ranges.len(), expected, "{:?}", parsed.diagnostics);
    assert!(parsed.diagnostics.ok);
}

fn rejected_by_both(input: Value) {
    assert!(
        !validate(&compress_parameters(), &input),
        "schema wrongly accepted: {input}"
    );
    assert!(parse_compress_args(&input, None).ranges.is_empty());
}

#[test]
fn the_flat_form_and_string_content_are_accepted_by_both_layers() {
    accepted_by_both(
        json!({ "topic": "T", "startId": "m00150", "endId": "m00220", "summary": "S" }),
        1,
    );
    accepted_by_both(json!({ "content": "m00150–m00220 T\nS body" }), 1);
}

#[test]
fn every_advertised_shape_passes_both_layers() {
    accepted_by_both(json!({ "content": "m00150–m00220 T\nS body" }), 1);
    accepted_by_both(
        json!({ "content": "m00150–m00160 A\nfirst\nm00170–m00220 B\nsecond" }),
        2,
    );
    accepted_by_both(
        json!({ "startRef": "m00150", "endRef": "m00220", "summary": "S" }),
        1,
    );
    let stringified = json!([{ "startId": "m00001", "endId": "m00002", "summary": "S" }]);
    accepted_by_both(json!({ "content": stringified.to_string() }), 1);
}

#[test]
fn structural_violations_are_rejected_by_both_layers() {
    rejected_by_both(json!({ "content": 42 }));
    rejected_by_both(json!({ "content": [{ "startId": "m1" }] }));
    rejected_by_both(json!({ "content": [42] }));
}

#[test]
fn empty_calls_pass_the_open_schema_and_the_parser_refuses_them() {
    for input in [
        json!({}),
        json!({ "topic": "x" }),
        json!({ "summary": "s" }),
    ] {
        assert!(validate(&compress_parameters(), &input), "{input}");
        assert!(
            parse_compress_args(&input, None).ranges.is_empty(),
            "{input}"
        );
    }
}

#[test]
fn an_object_under_content_is_refused_by_the_schema_and_salvaged_by_the_parser() {
    let trap = json!({ "content": { "startId": "m00150", "endId": "m00220", "summary": "S" } });
    assert!(!validate(&compress_parameters(), &trap));
    let parsed = parse_compress_args(&trap, None);
    assert_eq!(parsed.ranges.len(), 1);
    assert!(parsed.diagnostics.ok);
    assert!(parsed.diagnostics.content_salvage);
}

#[test]
fn an_empty_content_array_is_refused_by_the_schema_and_folds_nothing() {
    let input = json!({ "content": [] });
    assert!(!validate(&compress_parameters(), &input));
    assert!(parse_compress_args(&input, None).ranges.is_empty());
}

#[test]
fn the_content_description_teaches_the_line_form_and_the_string_form() {
    let parameters = compress_parameters();
    let description = parameters["properties"]["content"]["description"]
        .as_str()
        .unwrap();
    assert!(description.contains("m00150–m00220"));
    assert!(description.contains("JSON-encoded array"));
}

#[test]
fn the_schema_admits_exactly_the_string_form() {
    let parameters = compress_parameters();
    let properties: &Map<String, Value> = parameters["properties"].as_object().unwrap();
    assert_eq!(properties["content"]["type"], "string");
    assert!(properties["content"].get("anyOf").is_none());
    assert!(!properties.contains_key("topic"));
    let text = parameters.to_string();
    assert!(!text.contains("anyOf"));
    assert!(text.contains("m00150–m00220"));
    for input in [
        json!({ "topic": "x" }),
        json!({ "startId": "m1", "endId": "m2", "summary": "s" }),
        json!({ "content": "m00150–m00220 t\ns", "extra": 1 }),
    ] {
        assert!(validate(&parameters, &input), "{input}");
    }
}

#[test]
fn legacy_array_content_is_honoured_by_the_parser_and_not_advertised() {
    let legacy = [
        (json!({ "content": ["m00150-m00220\nS body"] }), 1),
        (
            json!({ "content": [{ "startId": "m00150", "endId": "m00220", "summary": "S" }] }),
            1,
        ),
        (
            json!({ "content": [
                "m00150-m00160 A\nfirst",
                { "startId": "m00170", "endId": "m00220", "summary": "second", "topic": "B" },
            ] }),
            2,
        ),
        (
            json!({ "content": [{ "startRef": "m00150", "endRef": "m00220", "summary": "S" }] }),
            1,
        ),
    ];
    for (input, expected) in legacy {
        assert!(!validate(&compress_parameters(), &input), "{input}");
        assert_eq!(
            parse_compress_args(&input, None).ranges.len(),
            expected,
            "{input}"
        );
    }
}
