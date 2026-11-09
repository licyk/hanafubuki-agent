// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `tests/fork-adoption-plan.test.ts` at commit `4be21b08e`. Copyright (c)
// 2026 ranxianglei. MIT License with an additional attribution term
// (NOTICE).
// Ports the plan's cases through `adopt_fork_state`, which always adopts
// blocks and keeps every present ref (the kernel's `seedAllRefs`), so the
// cases of the options turned off are not ported.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    adopt_fork_state, assign_message_refs, sync_blocks, Block, State, BLOCKED_REF, REF_FLOOR_ID,
};

use crate::fixtures::{assistant, config, user};

fn present<'a>(ids: &[&'a str]) -> HashSet<&'a str> {
    ids.iter().copied().collect()
}

fn block(block_id: &str) -> Block {
    Block {
        block_id: block_id.to_owned(),
        run_id: "r1".to_owned(),
        summary: "saved summary".to_owned(),
        direct_message_ids: vec!["raw1".to_owned(), "raw2".to_owned()],
        effective_message_ids: vec!["raw1".to_owned(), "raw2".to_owned()],
        compressed_tokens: 2_000,
        created_at: 1,
        ..Block::default()
    }
}

fn parent(blocks: Vec<Block>) -> State {
    State {
        blocks,
        next_block_id: 3,
        next_run_id: 2,
        ..State::default()
    }
}

/// Eight messages numbered in order; `b1` folds the first three.
fn numbered_parent() -> State {
    let mut state = parent(vec![Block {
        effective_message_ids: vec!["raw1".to_owned(), "raw2".to_owned(), "raw3".to_owned()],
        ..block("b1")
    }]);
    for index in 1..=8 {
        let (raw, r#ref) = (format!("raw{index}"), format!("m{index:05}"));
        state.message_refs.by_raw.insert(raw.clone(), r#ref.clone());
        state.message_refs.by_ref.insert(r#ref.clone(), raw);
        state.token_snapshot.insert(r#ref, 10 * index);
    }
    state
}

#[test]
fn present_refs_keep_their_numbers_and_fresh_ones_start_above_the_parent() {
    // raw4 is not in the fork: its number is never issued again.
    let parent = numbered_parent();
    let fork = adopt_fork_state(
        &parent,
        &present(&["raw1", "raw2", "raw3", "raw5", "raw6", "raw7", "raw8"]),
    );
    let refs = &fork.message_refs;
    assert_eq!(refs.by_raw["raw5"], "m00005");
    assert_eq!(refs.by_raw["raw8"], "m00008");
    assert!(!refs.by_raw.contains_key("raw4"));
    assert!(!refs.by_ref.contains_key("m00004"));
    assert_eq!(fork.token_snapshot["m00005"], 50);
    assert!(!fork.token_snapshot.contains_key("m00004"));
    assert_eq!(fork.blocks.len(), 1);
    assert!(fork.blocks[0].active);
}

#[test]
fn the_parent_ref_space_is_reserved_when_its_tail_is_dropped() {
    let parent = numbered_parent();
    let mut fork = adopt_fork_state(&parent, &present(&["raw1", "raw2", "raw3"]));
    assert_eq!(fork.message_refs.by_raw[REF_FLOOR_ID], "m00008");
    assert!(!fork.message_refs.by_ref.contains_key("m00008"));

    let messages = [user("fresh", "a message the fork adds")];
    assign_message_refs(&mut fork, &messages, &config());
    assert_eq!(fork.message_refs.by_raw["fresh"], "m00009");
}

#[test]
fn blocked_refs_never_enter_by_ref() {
    let mut parent = parent(Vec::new());
    for id in ["p1", "p2"] {
        parent
            .message_refs
            .by_raw
            .insert(id.to_owned(), BLOCKED_REF.to_owned());
    }
    parent
        .message_refs
        .by_raw
        .insert("r1".to_owned(), "m00001".to_owned());
    parent
        .message_refs
        .by_ref
        .insert("m00001".to_owned(), "r1".to_owned());
    let fork = adopt_fork_state(&parent, &present(&["p1", "p2", "r1"]));
    assert_eq!(fork.message_refs.by_raw["p1"], BLOCKED_REF);
    assert_eq!(fork.message_refs.by_ref["m00001"], "r1");
    assert!(!fork.message_refs.by_ref.contains_key(BLOCKED_REF));
}

#[test]
fn a_fully_present_fold_the_last_view_deactivated_is_restored() {
    let mut parent = parent(vec![block("b1")]);
    sync_blocks(&[user("side", "short view")], &mut parent);
    assert!(!parent.blocks[0].active);
    let before = parent.clone();
    let fork = adopt_fork_state(&parent, &present(&["raw1", "raw2"]));
    assert_eq!(fork.blocks.len(), 1);
    assert!(fork.blocks[0].active);
    assert_eq!(parent, before);
}

#[test]
fn expanded_consumed_and_straddling_blocks_are_not_adopted() {
    let inactive = Block {
        active: false,
        ..block("b1")
    };
    let expanded = Block {
        expanded: true,
        ..inactive.clone()
    };
    let none = |state: &State| state.active_blocks().count();
    assert_eq!(
        none(&adopt_fork_state(
            &parent(vec![expanded]),
            &present(&["raw1", "raw2"])
        )),
        0
    );
    let straddling = adopt_fork_state(&parent(vec![inactive.clone()]), &present(&["raw1"]));
    assert!(straddling.blocks.is_empty());
    assert!(
        adopt_fork_state(&parent(vec![inactive.clone()]), &present(&["unrelated"]))
            .blocks
            .is_empty()
    );

    let tier = Block {
        tier: 2,
        direct_block_ids: vec!["b1".to_owned()],
        active: false,
        ..block("b2")
    };
    let fork = adopt_fork_state(&parent(vec![inactive, tier]), &present(&["raw1", "raw2"]));
    assert_eq!(fork.active_blocks().count(), 1);
    assert!(!fork.block("b1").unwrap().active);
    assert!(fork.block("b2").unwrap().active);
}

#[test]
fn ids_and_latches_carry_over_while_the_nudge_and_stats_start_fresh() {
    let mut parent = parent(vec![block("b1")]);
    parent.hidden_orphan_refs = vec!["m00002".to_owned()];
    parent.swept_orphan_call_ids = vec!["call-9".to_owned()];
    parent.render_deferred_refs = vec!["m00001".to_owned()];
    parent.terminal_streak = 2;
    parent.stats.compression_count = 4;
    parent.nudge.last_nudge_shown_tokens = 40_000;
    let fork = adopt_fork_state(&parent, &present(&["raw1", "raw2"]));
    assert_eq!(fork.next_block_id, 3);
    assert_eq!(fork.next_run_id, 2);
    assert_eq!(fork.hidden_orphan_refs, parent.hidden_orphan_refs);
    assert_eq!(fork.swept_orphan_call_ids, parent.swept_orphan_call_ids);
    assert_eq!(fork.render_deferred_refs, parent.render_deferred_refs);
    assert_eq!(fork.terminal_streak, 0);
    assert_eq!(fork.stats, State::default().stats);
    assert_eq!(fork.nudge, State::default().nudge);
}

#[test]
fn an_adopted_fold_projects_in_the_fork() {
    let mut parent = parent(vec![block("b1")]);
    let messages = [
        user("raw1", "question"),
        assistant("raw2", "answer"),
        user("raw3", "next"),
    ];
    sync_blocks(&messages, &mut parent);
    let mut fork = adopt_fork_state(&parent, &present(&["raw1", "raw2", "raw3"]));
    sync_blocks(&messages, &mut fork);
    assert!(fork.block("b1").unwrap().active);
}
