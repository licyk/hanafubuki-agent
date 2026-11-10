// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports token-snapshot (render), issue3945-id-collision,
// issue3939-ref-migration-defer and the tag shape of render-refs.

use std::collections::BTreeMap;

use hanafubuki_agent_compression::{
    apply_compression, assign_message_refs, assign_refs, default_count_tokens, prune, render_refs,
    sync_blocks, ApplyInput, Config, Message, RefMap, RenderStrategy, State, BLOCKED_REF,
};

use crate::fixtures::{apply, assistant, call, config, range, result, state_for, user};

fn tokens_of(text: &str) -> Option<&str> {
    let start = text.find("tokens=\"")? + "tokens=\"".len();
    let end = start + text[start..].find('"')?;
    Some(&text[start..end])
}

#[test]
fn a_tag_carries_the_ref_its_frozen_tokens_and_the_message_type() {
    let messages = vec![user("a", "hello world")];
    let mut state = state_for(&messages);
    let rendered = render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    let tokens = default_count_tokens("hello world");
    assert_eq!(
        rendered[0].text,
        format!("<acp tokens=\"{tokens}\" type=\"text\">m00001</acp>\nhello world")
    );
    assert_eq!(
        state.token_snapshot,
        BTreeMap::from([("m00001".to_owned(), tokens)])
    );
}

#[test]
fn a_count_is_a_cjk_character_or_a_quarter_of_any_other_utf16_unit() {
    assert_eq!(default_count_tokens(""), 0);
    assert_eq!(default_count_tokens("hello"), 2);
    assert_eq!(default_count_tokens("café"), 1);
    assert_eq!(default_count_tokens("🙂🙂🙂"), 2);
    assert_eq!(default_count_tokens("中文 ab"), 3);
}

#[test]
fn the_snapshot_keeps_tags_stable_whatever_the_count_would_be_now() {
    let messages = vec![
        user("a", "你好世界这是一段中文消息"),
        user("b", "hello world"),
    ];
    let mut state = state_for(&messages);
    state.token_snapshot = BTreeMap::from([("m00001".to_owned(), 99), ("m00002".to_owned(), 7)]);
    let snapshot = state.token_snapshot.clone();
    let first = render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    let second = render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    let tags: Vec<Option<&str>> = first.iter().map(|m| tokens_of(&m.text)).collect();
    assert_eq!(tags, [Some("99"), Some("7")]);
    assert_eq!(first, second);
    assert_eq!(state.token_snapshot, snapshot, "a hit changes nothing");
}

#[test]
fn rendering_a_rendered_message_again_strips_its_own_tag_only() {
    let messages = vec![user(
        "a",
        "<acp tokens=\"3\" type=\"text\">m00042</acp>\nquoted foreign tag",
    )];
    let mut state = state_for(&messages);
    let once = render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    let twice = render_refs(&once, &mut state, RenderStrategy::TextOnly, false);
    assert_eq!(once, twice);
    assert!(twice[0].text.starts_with("<acp tokens="));
    assert!(twice[0]
        .text
        .contains(">m00001</acp>\n<acp tokens=\"3\" type=\"text\">m00042</acp>"));
}

#[test]
fn a_state_without_a_snapshot_decodes_and_renders() {
    let messages = vec![user("a", "hello world legacy state")];
    let mut legacy = state_for(&messages);
    let mut json = serde_json::to_value(&legacy).expect("serializes");
    json.as_object_mut()
        .expect("an object")
        .remove("token_snapshot");
    legacy = serde_json::from_value(json).expect("decodes without the field");
    assert!(legacy.token_snapshot.is_empty());
    let rendered = render_refs(&messages, &mut legacy, RenderStrategy::TextOnly, false);
    assert_eq!(legacy.token_snapshot.len(), 1);
    assert!(rendered[0].text.contains("tokens=\""));
}

#[test]
fn a_fold_carries_the_snapshot_through() {
    let messages = vec![
        user("a", "first message content here"),
        user("b", "second message content"),
    ];
    let mut state = state_for(&messages);
    render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    let snapshot = state.token_snapshot.clone();
    let outcome = apply(
        &[range("m00002", "m00002", "compressed")],
        &messages,
        &state,
        &config(),
    );
    assert_eq!(outcome.result.blocks_created, 1, "{:?}", outcome.result);
    assert_eq!(outcome.state.token_snapshot, snapshot);
}

#[test]
fn token_counts_format_as_javascript_rounds_them() {
    let messages: Vec<Message> = (1..=7).map(|i| user(&format!("t{i}"), "x")).collect();
    let mut state = state_for(&messages);
    let counts = [999, 1250, 1150, 1750, 9_999, 10_000, 12_500];
    for (index, count) in counts.iter().enumerate() {
        state
            .token_snapshot
            .insert(format!("m{:05}", index + 1), *count);
    }
    let rendered = render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    let tags: Vec<&str> = rendered.iter().filter_map(|m| tokens_of(&m.text)).collect();
    assert_eq!(tags, ["999", "1.3K", "1.1K", "1.8K", "10.0K", "10K", "13K"]);
}

#[test]
fn the_strategies_tag_text_results_or_nothing_but_never_call_arguments() {
    let messages = vec![
        user("u", "question"),
        call("c", "call-1", "read", "{\"path\":\"a\"}"),
        result("r", "call-1", "read", "contents"),
    ];
    let mut state = state_for(&messages);
    let tagged = |rendered: &[Message]| -> Vec<bool> {
        rendered
            .iter()
            .map(|m| m.text.starts_with("<acp "))
            .collect()
    };

    let text_only = render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    assert_eq!(tagged(&text_only), [true, false, false]);
    let with_results = render_refs(
        &messages,
        &mut state,
        RenderStrategy::TextAndToolResults,
        false,
    );
    assert_eq!(tagged(&with_results), [true, false, true]);
    assert!(with_results[2].text.contains("type=\"read\""));
    assert_eq!(with_results[1].text, "{\"path\":\"a\"}");
    let none = render_refs(&messages, &mut state, RenderStrategy::None, false);
    assert_eq!(none, messages);
}

const SHARED_PROTECTED: &str = "shared id, protected";
const SHARED_UNPROTECTED: &str = "shared id, NOT protected";

fn collision() -> Vec<Message> {
    vec![
        user("C", SHARED_PROTECTED),
        user("D", "distinct id"),
        user("C", SHARED_UNPROTECTED),
    ]
}

fn protected_first(message: &Message) -> bool {
    message.text == SHARED_PROTECTED
}

#[test]
fn an_unprotected_sibling_never_steals_a_blocked_binding() {
    let messages = collision();
    let assigned = assign_refs(&messages, &RefMap::default(), 1, protected_first);
    assert_eq!(assigned.map.by_raw["C"], BLOCKED_REF);
    assert!(assigned.migrated_ids.is_empty());
    let mut state = State {
        message_refs: assigned.map,
        ..State::default()
    };
    let rendered = render_refs(&messages, &mut state, RenderStrategy::TextOnly, false);
    assert_eq!(rendered[0].text, SHARED_PROTECTED);
    assert_eq!(rendered[2].text, SHARED_UNPROTECTED);
}

#[test]
fn bytes_stay_stable_round_over_round_under_the_collision() {
    let messages = collision();
    let mut state = State::default();
    let mut next = 1;
    let mut previous: Option<Vec<String>> = None;
    for _ in 0..3 {
        let assigned = assign_refs(&messages, &state.message_refs, next, protected_first);
        state.message_refs = assigned.map;
        next = assigned.next_index;
        let texts: Vec<String> =
            render_refs(&messages, &mut state, RenderStrategy::TextOnly, false)
                .into_iter()
                .map(|m| m.text)
                .collect();
        if let Some(previous) = &previous {
            assert_eq!(&texts, previous);
        }
        previous = Some(texts);
    }
    let texts = previous.expect("three rounds");
    assert!(texts[1].starts_with("<acp"));
    assert_eq!(texts[0], SHARED_PROTECTED);
    assert_eq!(texts[2], SHARED_UNPROTECTED);
}

#[test]
fn migration_fires_once_every_same_id_message_is_unprotected() {
    let first = assign_refs(
        &[user("C", "shared id, was protected")],
        &RefMap::default(),
        1,
        |_| true,
    );
    assert_eq!(first.map.by_raw["C"], BLOCKED_REF);
    let second = assign_refs(
        &[
            user("C", "shared id, was protected"),
            user("C", "shared id, never protected"),
        ],
        &first.map,
        first.next_index,
        |_| false,
    );
    assert_eq!(second.map.by_raw["C"], "m00001");
    assert_eq!(second.migrated_ids, ["C"]);
}

#[test]
fn unprotected_duplicate_ids_mint_one_ref() {
    let assigned = assign_refs(
        &[user("C", "first"), user("C", "second")],
        &RefMap::default(),
        1,
        |_| false,
    );
    assert_eq!(assigned.map.by_raw["C"], "m00001");
    assert_eq!(assigned.map.by_ref["m00001"], "C");
    assert_eq!(assigned.newly_assigned, 1);
}

fn migration_config() -> Config {
    let mut config = Config::new(100_000);
    config.preserve_recent_tokens = 100;
    config
}

fn history() -> Vec<Message> {
    let filler = "filler ".repeat(340);
    vec![
        user("raw-a", &format!("first question {filler}")),
        user("raw-img", &format!("look at this {filler}")),
        assistant("raw-b", &format!("answer {filler}")),
        user("raw-c", "second question"),
        assistant("raw-d", "second answer"),
        user("raw-e", "third question"),
        assistant("raw-f", "third answer"),
    ]
}

/// The view steps the kernel's request pipeline runs that these tests rely
/// on: refs, block sync, prune, tags.
fn view(messages: &[Message], state: &mut State, config: &Config) -> Vec<Message> {
    assign_message_refs(state, messages, config);
    sync_blocks(messages, state);
    let pruned = prune(messages, state);
    render_refs(&pruned, state, RenderStrategy::TextOnly, false)
}

fn has_tag(messages: &[Message], id: &str) -> Option<bool> {
    messages
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.text.contains("<acp "))
}

#[test]
fn a_migrated_ref_is_minted_but_its_tag_waits() {
    let config = migration_config();
    let mut state = State::default();
    state
        .message_refs
        .by_raw
        .insert("raw-img".to_owned(), BLOCKED_REF.to_owned());
    let first = view(&history(), &mut state, &config);
    let image_ref = state.message_refs.by_raw["raw-img"].clone();
    assert_ne!(image_ref, BLOCKED_REF);
    assert_eq!(has_tag(&first, "raw-img"), Some(false));
    assert_eq!(state.render_deferred_refs, [image_ref]);
    assert_eq!(has_tag(&first, "raw-a"), Some(true));

    let second = view(&history(), &mut state, &config);
    assert_eq!(
        has_tag(&second, "raw-img"),
        Some(false),
        "the deferral persists on quiet turns"
    );
}

#[test]
fn a_fold_releases_the_deferral() {
    let config = migration_config();
    let mut state = State::default();
    state
        .message_refs
        .by_raw
        .insert("raw-img".to_owned(), BLOCKED_REF.to_owned());
    view(&history(), &mut state, &config);
    assert!(!state.render_deferred_refs.is_empty());

    let start = state.message_refs.by_raw["raw-a"].clone();
    let end = state.message_refs.by_raw["raw-b"].clone();
    let spec = range(
        &start,
        &end,
        "a long enough summary for the fold gate to accept this range, over fifty chars",
    );
    let fold = apply_compression(&ApplyInput {
        ranges: &[spec],
        messages: &history(),
        state: &state,
        config: &config,
        protected_refs: None,
        now_ms: 0,
    });
    assert_eq!(fold.result.blocks_created, 1, "{:?}", fold.result);
    assert!(fold.state.render_deferred_refs.is_empty());

    let mut state = fold.state;
    let third = view(&history(), &mut state, &config);
    assert_ne!(
        has_tag(&third, "raw-img"),
        Some(false),
        "folded away or tagged at the fold"
    );
}
