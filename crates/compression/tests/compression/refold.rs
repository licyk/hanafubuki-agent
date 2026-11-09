// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).

// Ports `refold.test.ts` (refold in place; the inline-decompress marker is
// set on the block directly until the decompress module is ported).

use hanafubuki_agent_compression::{
    evaluate_refold, index_to_ref, Block, Config, Message, RefoldDecision, Role, State,
};

use crate::fixtures::{apply, config, range, with_topic};

const OLD_SUMMARY: &str = "OLD SUMMARY";
const NEW_SUMMARY: &str = "NEW SUMMARY carrying the re-folded details verbatim.";

fn refold_config() -> Config {
    let mut config = config();
    config.compress.min_compress_range = 500;
    config.compress.max_summary_length = 20_000;
    config.compress.min_summary_length = 10;
    config
}

fn make_messages(count: usize, filler: usize) -> Vec<Message> {
    (1..=count)
        .map(|index| {
            Message::text(
                format!("msg-{index}"),
                Role::Assistant,
                format!("content {index} {}", "x".repeat(filler)),
            )
        })
        .collect()
}

fn short_messages(count: usize) -> Vec<Message> {
    make_messages(count, 20)
}

fn long_messages(count: usize) -> Vec<Message> {
    make_messages(count, 80)
}

fn message_ids(messages: &[Message]) -> Vec<String> {
    messages.iter().map(|message| message.id.clone()).collect()
}

fn make_block(block_id: &str, effective: Vec<String>) -> Block {
    Block {
        block_id: block_id.to_owned(),
        run_id: "r1".to_owned(),
        tier: 1,
        summary: OLD_SUMMARY.to_owned(),
        direct_message_ids: effective.clone(),
        effective_message_ids: effective,
        compressed_tokens: 100,
        created_at: 1_758_800_000_000,
        ..Block::default()
    }
}

fn spanning(mut block: Block, start: &str, end: &str) -> Block {
    block.start_ref = Some(start.to_owned());
    block.end_ref = Some(end.to_owned());
    block
}

/// Refs over every message; the blocks fold the early ones.
fn make_state(messages: &[Message], blocks: Vec<Block>) -> State {
    let mut state = State::default();
    for (index, message) in messages.iter().enumerate() {
        let r#ref = index_to_ref(index as u32 + 1).unwrap();
        state
            .message_refs
            .by_raw
            .insert(message.id.clone(), r#ref.clone());
        state.message_refs.by_ref.insert(r#ref, message.id.clone());
    }
    state.next_block_id = blocks
        .iter()
        .map(|block| block.block_id[1..].parse::<u32>().unwrap() + 1)
        .fold(state.next_block_id, u32::max);
    state.blocks = blocks;
    state
}

/// What an inline decompress records on a block.
fn restored(mut state: State, block_id: &str) -> State {
    state
        .blocks
        .iter_mut()
        .find(|block| block.block_id == block_id)
        .unwrap()
        .restored_inline = true;
    state
}

fn summary_anchor(block_id: &str) -> Message {
    Message::text(format!("acp_summary_{block_id}"), Role::System, OLD_SUMMARY)
}

#[test]
fn the_restored_inline_marker_survives_a_json_round_trip() {
    let messages = short_messages(4);
    let state = restored(
        make_state(&messages, vec![make_block("b3", message_ids(&messages))]),
        "b3",
    );
    let persisted: State = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert!(persisted.blocks[0].restored_inline);
}

#[test]
fn refold_updates_the_block_in_place() {
    let messages = short_messages(10);
    let ids = message_ids(&messages);
    let marked = restored(
        make_state(
            &messages,
            vec![spanning(make_block("b1", ids.clone()), "m00001", "m00010")],
        ),
        "b1",
    );
    let outcome = apply(
        &[with_topic(range("m00001", "m00010", NEW_SUMMARY), "refold")],
        &[],
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(
        outcome.state.blocks.len(),
        1,
        "no second block for the same span"
    );
    let block = &outcome.state.blocks[0];
    assert_eq!(block.block_id, "b1");
    assert_eq!(block.summary, NEW_SUMMARY);
    assert_eq!(block.topic.as_deref(), Some("refold"));
    assert!(!block.restored_inline);
    assert!(block.active);
    assert_eq!(block.tier, 1, "refold is not distillation");
    assert_eq!(block.direct_message_ids, ids);
    assert_eq!(block.effective_message_ids, ids);
    assert_eq!(outcome.state.next_block_id, 2, "no new block id allocated");
}

#[test]
fn partial_coverage_of_a_restored_block_no_ops_naming_the_blocking_block() {
    let messages = short_messages(10);
    let marked = restored(
        make_state(&messages, vec![make_block("b1", message_ids(&messages))]),
        "b1",
    );
    let outcome = apply(
        &[range("m00001", "m00008", NEW_SUMMARY)],
        &[],
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.noop);
    assert!(outcome
        .result
        .warnings
        .join(" ")
        .contains("partially covered b1"));
    let block = &outcome.state.blocks[0];
    assert_eq!(block.summary, OLD_SUMMARY, "no mutation on the no-op");
    assert!(block.restored_inline, "the marker survives the no-op");
    assert_eq!(outcome.state.next_block_id, 2);
}

fn nested(messages: &[Message]) -> State {
    let ids = message_ids(messages);
    let mut b1 = make_block("b1", ids.clone());
    b1.active = false;
    let mut b2 = make_block("b2", ids);
    b2.direct_block_ids = vec!["b1".to_owned()];
    b2.direct_message_ids = Vec::new();
    b2.tier = 2;
    make_state(messages, vec![b1, b2])
}

#[test]
fn an_unrecovered_higher_tier_block_above_a_restored_block_blocks_the_refold() {
    let messages = short_messages(10);
    let marked = restored(nested(&messages), "b1");
    let outcome = apply(
        &[range("m00001", "m00010", NEW_SUMMARY)],
        &[],
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(
        outcome.result.noop,
        "a blocked refold is the covered no-op class"
    );
    let warnings = outcome.result.warnings.join(" ");
    assert_eq!(
        warnings.matches("blocked by b2 (not restored)").count(),
        1,
        "{warnings}"
    );
    assert_eq!(outcome.state.blocks[0].summary, OLD_SUMMARY);
    assert!(outcome.state.blocks[0].restored_inline);
}

#[test]
fn restoring_the_topmost_active_tier_folds_back_into_exactly_that_block() {
    let messages = short_messages(10);
    let marked = restored(nested(&messages), "b2");
    let outcome = apply(
        &[range("m00001", "m00010", NEW_SUMMARY)],
        &[],
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(
        outcome.result.blocks_created, 1,
        "only the topmost active tier is refolded"
    );
    let b1 = outcome.state.block("b1").unwrap();
    let b2 = outcome.state.block("b2").unwrap();
    assert_eq!(b2.summary, NEW_SUMMARY);
    assert!(!b2.restored_inline);
    assert_eq!(
        b1.summary, OLD_SUMMARY,
        "the inactive lower tier is left untouched"
    );
    assert_eq!(outcome.state.next_block_id, 3);
}

fn siblings(messages: &[Message], second_from: usize) -> State {
    let state = make_state(
        messages,
        vec![
            make_block("b1", message_ids(&messages[..10])),
            make_block("b3", message_ids(&messages[second_from..])),
        ],
    );
    restored(restored(state, "b1"), "b3")
}

fn assert_all_refolded(outcome: &hanafubuki_agent_compression::ApplyOutcome) {
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 2);
    assert_eq!(outcome.state.next_block_id, 4, "no ids allocated");
    for block in &outcome.state.blocks {
        assert_eq!(block.summary, NEW_SUMMARY);
        assert!(!block.restored_inline);
    }
}

#[test]
fn two_sibling_restored_blocks_in_one_range_are_both_refolded_in_place() {
    let messages = short_messages(20);
    let state = siblings(&messages, 14);
    let outcome = apply(
        &[range("m00001", "m00020", NEW_SUMMARY)],
        &[],
        &state,
        &refold_config(),
    );
    assert_all_refolded(&outcome);
}

// Full-log views keep the restored originals visible: the range resolves
// instead of classifying as consumed, and takes the same refold path.

#[test]
fn full_anchor_view_refold_updates_the_block_in_place() {
    let messages = short_messages(10);
    let ids = message_ids(&messages);
    let marked = restored(
        make_state(
            &messages,
            vec![spanning(make_block("b1", ids.clone()), "m00001", "m00010")],
        ),
        "b1",
    );
    let mut view = vec![summary_anchor("b1")];
    view.extend(messages);
    let outcome = apply(
        &[with_topic(range("m00001", "m00010", NEW_SUMMARY), "refold")],
        &view,
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(outcome.state.blocks.len(), 1);
    let block = &outcome.state.blocks[0];
    assert_eq!(block.block_id, "b1");
    assert_eq!(block.summary, NEW_SUMMARY);
    assert_eq!(block.topic.as_deref(), Some("refold"));
    assert!(!block.restored_inline);
    assert!(block.active);
    assert_eq!(block.tier, 1);
    assert_eq!(block.direct_message_ids, ids);
    assert_eq!(block.effective_message_ids, ids);
    assert_eq!(outcome.state.next_block_id, 2);
}

#[test]
fn full_anchor_view_single_restored_block_with_a_live_tail_refolds_only_that_block() {
    let messages = short_messages(8);
    let first = vec![messages[0].id.clone()];
    let marked = restored(
        make_state(
            &messages,
            vec![spanning(
                make_block("b1", first.clone()),
                "m00001",
                "m00001",
            )],
        ),
        "b1",
    );
    let mut view = vec![summary_anchor("b1")];
    view.extend(messages);
    let outcome = apply(
        &[range("m00001", "m00001", NEW_SUMMARY)],
        &view,
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(
        outcome.state.blocks.len(),
        1,
        "the live tail is left untouched"
    );
    let block = &outcome.state.blocks[0];
    assert_eq!(block.block_id, "b1");
    assert_eq!(block.summary, NEW_SUMMARY);
    assert!(!block.restored_inline);
    assert_eq!(block.effective_message_ids, first);
    assert_eq!(outcome.state.next_block_id, 2);
}

#[test]
fn full_anchor_view_two_adjacent_restored_blocks_are_both_refolded_in_place() {
    let messages = short_messages(20);
    let state = siblings(&messages, 10);
    let mut view = vec![summary_anchor("b1"), summary_anchor("b3")];
    view.extend(messages);
    let outcome = apply(
        &[range("m00001", "m00020", NEW_SUMMARY)],
        &view,
        &state,
        &refold_config(),
    );
    assert_all_refolded(&outcome);
}

#[test]
fn full_anchor_view_a_restored_block_with_a_live_tail_becomes_a_fresh_block_consuming_it() {
    let messages = long_messages(12);
    let marked = restored(
        make_state(
            &messages,
            vec![make_block("b1", message_ids(&messages[..10]))],
        ),
        "b1",
    );
    let mut view = vec![summary_anchor("b1")];
    view.extend(messages.clone());
    let outcome = apply(
        &[range("m00001", "m00012", NEW_SUMMARY)],
        &view,
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let b1 = outcome.state.block("b1").unwrap();
    assert!(
        !b1.active,
        "the restored block is consumed by the fresh fold"
    );
    assert_eq!(b1.summary, OLD_SUMMARY);
    let fresh = outcome
        .state
        .blocks
        .iter()
        .find(|block| block.block_id != "b1")
        .unwrap();
    assert!(fresh.active);
    assert_eq!(
        fresh.direct_message_ids,
        message_ids(&messages[10..]),
        "only the live tail is direct"
    );
    assert_eq!(outcome.state.next_block_id, 3);
}

#[test]
fn full_anchor_view_an_unmarked_block_gets_the_covered_no_op_with_block_id_guidance() {
    let messages = long_messages(10);
    let state = make_state(
        &messages,
        vec![spanning(
            make_block("b1", message_ids(&messages)),
            "m00001",
            "m00010",
        )],
    );
    let mut view = vec![summary_anchor("b1")];
    view.extend(messages);
    let outcome = apply(
        &[range("m00001", "m00010", NEW_SUMMARY)],
        &view,
        &state,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.noop);
    let warnings = outcome.result.warnings.join(" ");
    assert!(
        warnings.contains("already covered by active block(s) b1"),
        "{warnings}"
    );
    assert!(warnings.contains("reference them by block ID (b1..b1)"));
    assert!(warnings.contains("do not resend this range"));
    assert_eq!(outcome.state.blocks.len(), 1);
    assert_eq!(outcome.state.blocks[0].summary, OLD_SUMMARY);
    assert_eq!(outcome.state.next_block_id, 2);
}

#[test]
fn full_anchor_view_a_sub_threshold_span_over_an_unmarked_block_names_the_blocker() {
    let messages = short_messages(10);
    let state = make_state(
        &messages,
        vec![spanning(
            make_block("b1", message_ids(&messages)),
            "m00001",
            "m00010",
        )],
    );
    let mut view = vec![summary_anchor("b1")];
    view.extend(messages);
    let outcome = apply(
        &[range("m00001", "m00010", NEW_SUMMARY)],
        &view,
        &state,
        &refold_config(),
    );
    assert_eq!(outcome.result.errors.len(), 1);
    let error = &outcome.result.errors[0];
    assert!(
        error.contains("Total compressible content too small"),
        "{error}"
    );
    assert!(error.contains("blocked by b1 (not restored)"));
    assert!(error.contains("decompress with inline:true"));
    assert!(error.find("too small") < error.find("Refold blocked"));
    assert_eq!(outcome.state.blocks.len(), 1);
    assert_eq!(outcome.state.blocks[0].summary, OLD_SUMMARY);
    assert_eq!(outcome.state.next_block_id, 2);
}

fn refold_plus_fresh(view: &[Message], marked: &State, messages: &[Message]) {
    let outcome = apply(
        &[
            range("m00001", "m00010", NEW_SUMMARY),
            range("m00011", "m00020", NEW_SUMMARY),
        ],
        view,
        marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 2);
    let b1 = outcome.state.block("b1").unwrap();
    assert_eq!(b1.summary, NEW_SUMMARY, "refolded in place");
    assert!(!b1.restored_inline);
    let fresh: Vec<&Block> = outcome
        .state
        .blocks
        .iter()
        .filter(|block| block.block_id != "b1")
        .collect();
    assert_eq!(fresh.len(), 1, "the fresh part folds into one new block");
    assert!(fresh[0].active);
    assert_eq!(fresh[0].effective_message_ids, message_ids(&messages[10..]));
    assert_eq!(outcome.state.next_block_id, 3);
}

#[test]
fn full_anchor_view_a_refoldable_span_plus_a_qualifying_fresh_part_applies_both() {
    let messages = long_messages(20);
    let marked = restored(
        make_state(
            &messages,
            vec![make_block("b1", message_ids(&messages[..10]))],
        ),
        "b1",
    );
    let mut view = vec![summary_anchor("b1")];
    view.extend(messages.clone());
    refold_plus_fresh(&view, &marked, &messages);
}

#[test]
fn a_block_without_the_marker_is_not_refolded_until_marked() {
    let messages = short_messages(10);
    let fresh = make_state(&messages, vec![make_block("b1", message_ids(&messages))]);
    // A state persisted before the marker existed reads it as false.
    let mut json: serde_json::Value = serde_json::to_value(&fresh).unwrap();
    json["blocks"][0]
        .as_object_mut()
        .unwrap()
        .remove("restored_inline");
    let loaded: State = serde_json::from_value(json).unwrap();
    assert!(!loaded.blocks[0].restored_inline);

    let rejected = apply(
        &[range("m00001", "m00010", NEW_SUMMARY)],
        &[],
        &loaded,
        &refold_config(),
    );
    assert!(
        rejected.result.errors.is_empty(),
        "{:?}",
        rejected.result.errors
    );
    assert_eq!(rejected.result.blocks_created, 0);
    assert!(rejected.result.noop);
    assert!(rejected
        .result
        .warnings
        .join(" ")
        .contains("blocked by b1 (not restored)"));
    assert_eq!(rejected.state.blocks[0].summary, OLD_SUMMARY);

    let accepted = apply(
        &[range("m00001", "m00010", NEW_SUMMARY)],
        &[],
        &restored(loaded, "b1"),
        &refold_config(),
    );
    assert!(
        accepted.result.errors.is_empty(),
        "{:?}",
        accepted.result.errors
    );
    assert_eq!(accepted.state.blocks[0].summary, NEW_SUMMARY);
}

#[test]
fn a_too_small_live_range_beside_a_restorable_range_keeps_the_gate() {
    let messages = short_messages(12);
    let marked = restored(
        make_state(
            &messages,
            vec![make_block("b1", message_ids(&messages[..10]))],
        ),
        "b1",
    );
    let outcome = apply(
        &[
            range("m00001", "m00010", NEW_SUMMARY),
            range("m00011", "m00012", NEW_SUMMARY),
        ],
        &messages[10..],
        &marked,
        &refold_config(),
    );
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(outcome.result.errors[0].contains("already compressed"));
    assert_eq!(outcome.state.blocks.len(), 1);
    assert_eq!(
        outcome.state.blocks[0].summary, OLD_SUMMARY,
        "nothing applies when the gate rejects"
    );
}

#[test]
fn a_fresh_part_meeting_the_minimum_applies_the_refold_and_the_fresh_fold() {
    let messages = long_messages(20);
    let marked = restored(
        make_state(
            &messages,
            vec![make_block("b1", message_ids(&messages[..10]))],
        ),
        "b1",
    );
    refold_plus_fresh(&messages[10..], &marked, &messages);
}

#[test]
fn a_block_ref_range_over_a_restored_hidden_block_refolds_in_place() {
    let messages = short_messages(10);
    let marked = restored(
        make_state(
            &messages,
            vec![spanning(
                make_block("b1", message_ids(&messages)),
                "m00001",
                "m00010",
            )],
        ),
        "b1",
    );
    let outcome = apply(
        &[range("b1", "b1", NEW_SUMMARY)],
        &[],
        &marked,
        &refold_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(outcome.state.blocks.len(), 1);
    let block = &outcome.state.blocks[0];
    assert_eq!(block.block_id, "b1");
    assert_eq!(block.summary, NEW_SUMMARY);
    assert!(!block.restored_inline);
    assert_eq!(outcome.state.next_block_id, 2);
}

#[test]
fn evaluate_refold_names_its_verdict_directly() {
    let messages = short_messages(10);
    let state = make_state(&messages, vec![make_block("b1", message_ids(&messages))]);
    assert_eq!(
        evaluate_refold(&state, "m00001", "m00010"),
        RefoldDecision::Blocked(vec!["blocked by b1 (not restored)".to_owned()])
    );
    let marked = restored(state, "b1");
    assert_eq!(
        evaluate_refold(&marked, "m00001", "m00010"),
        RefoldDecision::Refold(vec!["b1".to_owned()])
    );
    assert_eq!(
        evaluate_refold(&marked, "m00001", "m00008"),
        RefoldDecision::Blocked(vec!["partially covered b1".to_owned()])
    );
    assert_eq!(
        evaluate_refold(&marked, "foo", "m00008"),
        RefoldDecision::Blocked(Vec::new())
    );
}
