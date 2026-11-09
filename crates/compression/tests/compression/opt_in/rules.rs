// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `rules.test.ts`; adds `executeRule`'s operations (from
// billion-context's `src/rules-feature.ts`). Not ported: the
// `mergeCompressionState` case (every state field decodes with its default
// here, which the last case pins).

use serde_json::json;

use hanafubuki_agent_compression::{
    add_rule, allocate_rule_id, clear_rules, execute_rule, format_rules_for_prompt,
    format_rules_list, remove_rule, Config, RuleRecord, State, ALWAYS_PROTECTED_TOOLS,
    RULE_TOOL_NAME,
};

use crate::fixtures::{apply, call, range, state_for, user};

fn rule(id: &str, text: &str) -> RuleRecord {
    RuleRecord {
        id: id.to_owned(),
        text: text.to_owned(),
    }
}

fn config() -> Config {
    Config::new(200_000)
}

#[test]
fn acp_rule_is_always_protected() {
    assert_eq!(RULE_TOOL_NAME, "acp_rule");
    assert!(ALWAYS_PROTECTED_TOOLS.contains(&"acp_rule"));
}

#[test]
fn the_first_rule_is_rule1_and_trimmed() {
    let mut state = State::default();
    let added = add_rule(&mut state, "  always run tests before pushing  ", &config());
    assert_eq!(added, Ok(rule("rule1", "always run tests before pushing")));
    assert_eq!(
        state.rules,
        [rule("rule1", "always run tests before pushing")]
    );
    assert_eq!(state.next_rule_id, 2);
}

#[test]
fn an_empty_rule_is_refused() {
    let mut state = State::default();
    let error = add_rule(&mut state, "   ", &config()).unwrap_err();
    assert!(error.contains("empty"));
    assert!(state.rules.is_empty());
}

#[test]
fn a_rule_over_the_length_limit_is_refused() {
    let mut state = State::default();
    let error = add_rule(&mut state, &"y".repeat(301), &config()).unwrap_err();
    assert!(error.contains("301 chars exceeds the 300-char limit"));
    let mut longer = config();
    longer.rules.max_rule_chars = 400;
    assert!(add_rule(&mut state, &"y".repeat(301), &longer).is_ok());
}

#[test]
fn an_identical_rule_points_at_the_existing_one() {
    let mut state = State::default();
    add_rule(&mut state, "never force-push", &config()).unwrap();
    let error = add_rule(&mut state, "never force-push", &config()).unwrap_err();
    assert!(error.contains("rule1"));
    assert_eq!(state.rules.len(), 1);
}

#[test]
fn the_count_is_capped() {
    let mut state = State::default();
    let mut capped = config();
    capped.rules.max_rules = 2;
    for index in 0..2 {
        add_rule(&mut state, &format!("rule text {index}"), &capped).unwrap();
    }
    let error = add_rule(&mut state, "one too many", &capped).unwrap_err();
    assert!(error.contains("limit reached (2)"));
    assert_eq!(state.rules.len(), 2);
}

#[test]
fn ids_are_never_reissued() {
    let mut state = State::default();
    add_rule(&mut state, "a", &config()).unwrap();
    add_rule(&mut state, "b", &config()).unwrap();
    remove_rule(&mut state, "rule2").unwrap();
    assert_eq!(add_rule(&mut state, "c", &config()).unwrap().id, "rule3");
    clear_rules(&mut state);
    assert_eq!(add_rule(&mut state, "d", &config()).unwrap().id, "rule4");
}

#[test]
fn a_hand_made_state_moves_the_counter_past_its_highest_id() {
    let state = State {
        rules: vec![rule("rule7", "hand-crafted")],
        next_rule_id: 1,
        ..State::default()
    };
    assert_eq!(allocate_rule_id(&state), "rule8");
    assert_eq!(state.next_rule_id, 1, "allocating is pure");
}

#[test]
fn the_limits_default_to_the_kernels() {
    let rules = config().rules;
    assert!(!rules.enabled);
    assert_eq!((rules.max_rules, rules.max_rule_chars), (50, 300));
}

#[test]
fn removing_and_clearing_report() {
    let mut state = State::default();
    add_rule(&mut state, "keep", &config()).unwrap();
    add_rule(&mut state, "drop", &config()).unwrap();
    assert_eq!(
        remove_rule(&mut state, " rule2 "),
        Ok(rule("rule2", "drop"))
    );
    let error = remove_rule(&mut state, "rule99").unwrap_err();
    assert!(error.contains("no rule with id"));
    assert_eq!(clear_rules(&mut state), 1);
    assert!(state.rules.is_empty());
}

#[test]
fn the_prompt_section_costs_nothing_without_rules() {
    let mut state = State::default();
    assert_eq!(format_rules_for_prompt(&state), "");
    add_rule(&mut state, "always run tests", &config()).unwrap();
    assert_eq!(
        format_rules_for_prompt(&state),
        "# Persistent rules (recorded via acp_rule — kept across compression)\n- [rule1] always run tests"
    );
}

#[test]
fn the_listing_is_numbered() {
    let out = format_rules_list(&[rule("rule1", "first"), rule("rule2", "second")]);
    assert_eq!(out, "1. [rule1] first\n2. [rule2] second");
}

#[test]
fn malformed_limits_are_reported() {
    let mut zero = config();
    zero.rules.max_rules = 0;
    assert_eq!(zero.validate(), ["rules.maxRules must be >= 1"]);
    let mut short = config();
    short.rules.max_rule_chars = 0;
    assert_eq!(short.validate(), ["rules.maxRuleChars must be >= 1"]);
    let mut on = config();
    on.rules.enabled = true;
    assert!(on.validate().is_empty());
}

#[test]
fn acp_rule_exchanges_never_fold() {
    let long = "x".repeat(6000);
    let messages = vec![
        user("a", &long),
        call("b", "call1", "acp_rule", r#"{"rule":"always run tests"}"#),
        hanafubuki_agent_compression::Message {
            tool_name: None,
            ..crate::fixtures::result("c", "call1", "", "Recorded rule1: always run tests")
        },
        user("d", &long),
    ];
    let mut config = crate::fixtures::config();
    config.compress.max_summary_length = 0;
    let applied = apply(
        &[range(
            "m00001",
            "m00004",
            "A meaningful summary that captures the key information of the compressed range including file paths and decisions.",
        )],
        &messages,
        &state_for(&messages),
        &config,
    );
    assert_eq!(applied.result.blocks_created, 1);
    assert!(applied.result.errors.is_empty());
    let block = &applied.state.blocks[0];
    for id in ["b", "c"] {
        assert!(!block.direct_message_ids.contains(&id.to_owned()), "{id}");
        assert!(
            !block.effective_message_ids.contains(&id.to_owned()),
            "{id}"
        );
    }
    for id in ["a", "d"] {
        assert!(block.direct_message_ids.contains(&id.to_owned()), "{id}");
    }
}

#[test]
fn a_state_without_rules_decodes_with_fresh_defaults() {
    let state: State = serde_json::from_value(json!({"blocks": []})).unwrap();
    assert!(state.rules.is_empty());
    assert_eq!(state.next_rule_id, 1);
    let mut with_rules = State::default();
    add_rule(&mut with_rules, "persisted rule", &config()).unwrap();
    let decoded: State =
        serde_json::from_value(serde_json::to_value(&with_rules).unwrap()).unwrap();
    assert_eq!(decoded.rules, [rule("rule1", "persisted rule")]);
    assert_eq!(decoded.next_rule_id, 2);
}

// --- `executeRule` ---

#[test]
fn acp_rule_records_lists_removes_and_clears() {
    let mut state = State::default();
    let run = |arguments: serde_json::Value, state: &mut State| {
        execute_rule(&arguments, state, &config())
    };
    let listed = run(json!({}), &mut state);
    assert!(listed.ok);
    assert_eq!(listed.text, "No rules recorded.");
    let recorded = run(json!({"rule": " test first "}), &mut state);
    assert_eq!(
        (recorded.ok, recorded.text.as_str()),
        (true, "Recorded rule1: test first")
    );
    run(json!({"rule": "never force-push"}), &mut state);
    assert_eq!(
        run(json!({}), &mut state).text,
        "1. [rule1] test first\n2. [rule2] never force-push"
    );
    let removed = run(json!({"delete": "rule1"}), &mut state);
    assert_eq!(removed.text, "Removed rule1: test first");
    let missing = run(json!({"delete": "rule9"}), &mut state);
    assert!(!missing.ok);
    let two = run(json!({"rule": "x", "clear": true}), &mut state);
    assert!(!two.ok);
    assert!(two.text.starts_with("Use one operation per call"));
    assert_eq!(
        run(json!({"clear": true}), &mut state).text,
        "Cleared 1 rule(s)."
    );
    assert_eq!(
        run(json!({"clear": true}), &mut state).text,
        "No rules to clear."
    );
}
