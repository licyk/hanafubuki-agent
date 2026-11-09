// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the decompress primitives of `decompress-report.test.ts` and the
// restored-preview case of `truncate-surrogates.test.ts`; the state is
// updated in place, so the kernel's immutability check has no counterpart.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    build_restored_content_preview, collect_block_content, deactivate_blocks, expand_block,
    find_active_ancestor, find_blocks_overlapping_messages, parse_block_id_arg, sync_blocks, Block,
    State,
};

use crate::fixtures::{assistant, result, user};

const EMOJI: &str = "\u{1F980}";

fn block(block_id: &str, effective: &[&str]) -> Block {
    Block {
        block_id: block_id.to_owned(),
        run_id: "r0".to_owned(),
        summary: "summary".to_owned(),
        effective_message_ids: effective.iter().map(|id| (*id).to_owned()).collect(),
        created_at: 1000,
        ..Block::default()
    }
}

fn consuming(mut parent: Block, child: &str) -> Block {
    parent.direct_block_ids = vec![child.to_owned()];
    parent
}

fn inactive(mut block: Block) -> Block {
    block.active = false;
    block
}

fn state_with(blocks: Vec<Block>) -> State {
    State {
        blocks,
        ..State::default()
    }
}

#[test]
fn a_block_id_argument_takes_bn_or_a_plain_number() {
    assert_eq!(parse_block_id_arg("b3").as_deref(), Some("b3"));
    assert_eq!(parse_block_id_arg("3").as_deref(), Some("b3"));
    assert_eq!(parse_block_id_arg("b005").as_deref(), Some("b5"));
    assert_eq!(parse_block_id_arg(" B5 ").as_deref(), Some("b5"));
    assert_eq!(parse_block_id_arg("garbage"), None);
}

#[test]
fn overlapping_blocks_are_the_active_ones_covering_a_message() {
    let state = state_with(vec![
        block("b1", &["m1", "m2"]),
        block("b2", &["m3"]),
        inactive(block("b3", &["m2"])),
    ]);
    let found = find_blocks_overlapping_messages(&state, &HashSet::from(["m2"]));
    let ids: Vec<&str> = found.iter().map(|block| block.block_id.as_str()).collect();
    assert_eq!(ids, ["b1"]);
}

#[test]
fn the_active_ancestor_is_the_nearest_active_block_down_the_lineage() {
    let state = state_with(vec![
        block("b1", &[]),
        inactive(consuming(block("b2", &[]), "b1")),
        consuming(block("b3", &[]), "b2"),
    ]);
    assert_eq!(find_active_ancestor(&state, "b3").as_deref(), Some("b1"));
    assert_eq!(find_active_ancestor(&state, "b2").as_deref(), Some("b1"));
}

#[test]
fn deactivating_a_block_leaves_the_others_active() {
    let mut state = state_with(vec![block("b1", &[]), block("b2", &[])]);
    deactivate_blocks(&mut state, &["b1"], false);
    assert!(!state.blocks[0].active);
    assert!(state.blocks[1].active);
}

#[test]
fn a_deep_deactivation_cascades_down_the_lineage() {
    let mut state = state_with(vec![block("b1", &[]), consuming(block("b2", &[]), "b1")]);
    deactivate_blocks(&mut state, &["b2"], true);
    assert!(!state.blocks[0].active);
    assert!(!state.blocks[1].active);
}

#[test]
fn an_expanded_block_stays_inactive_across_a_sync() {
    let messages = [user("m1", "hello"), assistant("m2", "world")];
    let mut state = state_with(vec![
        block("b1", &["m1"]),
        consuming(block("b2", &["m1", "m2"]), "b1"),
    ]);
    assert!(expand_block(&mut state, "b2"));
    assert!(!expand_block(&mut state, "b9"));
    sync_blocks(&messages, &mut state);
    assert!(state.blocks.iter().all(|block| !block.active));
    assert!(state.block("b2").unwrap().expanded);
    assert!(state.covered_message_ids().is_empty());
}

#[test]
fn the_restored_preview_lists_messages_no_longer_covered() {
    let messages = [
        user("m1", "hello world"),
        result("m2", "c1", "bash", "output"),
    ];
    let preview =
        build_restored_content_preview(&messages, &HashSet::from(["m1", "m2"]), &State::default());
    assert_eq!(preview.restored_count, 2);
    assert_eq!(preview.preview, "[user] hello world\nbash: output");
}

#[test]
fn a_restored_preview_cut_through_an_astral_character_drops_it() {
    let messages = [assistant("m1", &format!("{}{EMOJI}tail", "z".repeat(199)))];
    let preview =
        build_restored_content_preview(&messages, &HashSet::from(["m1"]), &State::default());
    assert_eq!(preview.restored_count, 1);
    assert_eq!(
        preview.preview,
        format!("[assistant] {}...", "z".repeat(199))
    );
}

/// `m1..m4`, `b1` (`intro`) folding `m1`, `m2` under `b2`.
fn nested_history() -> ([hanafubuki_agent_compression::Message; 4], State) {
    let messages = [
        user("m1", "hello"),
        result("m2", "c1", "read", "file a"),
        assistant("m3", "found bug"),
        user("m4", "direct message"),
    ];
    let mut child = block("b1", &["m1", "m2"]);
    child.summary = "child summary".to_owned();
    child.topic = Some("intro".to_owned());
    let parent = consuming(block("b2", &["m1", "m2", "m3", "m4"]), "b1");
    (messages, state_with(vec![child, parent]))
}

#[test]
fn one_tier_up_shows_an_active_child_s_summary_and_the_direct_messages() {
    let (messages, state) = nested_history();
    let collected = collect_block_content(&state, state.block("b2").unwrap(), &messages, false);
    assert_eq!(collected.count, 3);
    assert_eq!(
        collected.text,
        "[Compressed conversation section] — b1: intro\nchild summary\n\n\
         [assistant]\nfound bug\n\n[user]\ndirect message"
    );
}

#[test]
fn full_recurses_to_every_original_message() {
    let (messages, state) = nested_history();
    let collected = collect_block_content(&state, state.block("b2").unwrap(), &messages, true);
    assert_eq!(collected.count, 4);
    assert_eq!(
        collected.text,
        "[user]\nhello\n\n[tool • read]\nfile a\n\n[assistant]\nfound bug\n\n[user]\ndirect message"
    );
}

#[test]
fn one_tier_up_shows_an_inactive_child_s_messages_directly() {
    let messages = [user("m1", "hello"), assistant("m2", "world")];
    let mut child = inactive(block("b1", &["m1"]));
    child.summary = "inactive child".to_owned();
    let state = state_with(vec![child, consuming(block("b2", &["m1", "m2"]), "b1")]);
    let collected = collect_block_content(&state, state.block("b2").unwrap(), &messages, false);
    assert_eq!(collected.count, 2);
    assert!(collected.text.contains("hello"));
    assert!(collected.text.contains("world"));
    assert!(!collected.text.contains("inactive child"));
}

#[test]
fn an_empty_block_collects_nothing() {
    let state = state_with(vec![block("b1", &[])]);
    let collected = collect_block_content(&state, &state.blocks[0], &[], false);
    assert_eq!(collected.count, 0);
    assert_eq!(collected.text, "");
}
