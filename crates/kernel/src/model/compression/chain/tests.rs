use std::sync::Arc;

use hanafubuki_agent_compression::{Block, LocalTime, BLOCKED_REF, REF_FLOOR_ID};

use super::*;

/// `state` one request later: `count` messages minted (every third one
/// protected) and stamped, one block folded over the first two of them,
/// every active block a request older, the nudge moved on.
fn grown(state: &AgentCompressionState, count: usize) -> AgentCompressionState {
    let mut next = state.clone();
    let start = next.message_refs.by_raw.len();
    let mut ids = Vec::new();
    for index in start..start + count {
        let id = format!("1:{index}");
        let r#ref = if index % 3 == 2 {
            BLOCKED_REF.to_owned()
        } else {
            let r#ref = format!("m{:05}", index + 1);
            next.message_refs.by_ref.insert(r#ref.clone(), id.clone());
            next.token_snapshot.insert(r#ref.clone(), index as u64);
            r#ref
        };
        next.message_refs.by_raw.insert(id.clone(), r#ref);
        next.ts_stamps.insert(
            id.clone(),
            LocalTime {
                at: 1_000 * index as u64,
                utc_offset_seconds: 3_600,
            },
        );
        ids.push(id);
    }
    next.advance_survival(4);
    let block_id = format!("b{}", next.next_block_id);
    next.next_block_id += 1;
    next.blocks.push(Block {
        block_id,
        summary: format!("The messages from {start} on were read."),
        effective_message_ids: ids[..2].to_vec(),
        direct_message_ids: ids[..2].to_vec(),
        ..Block::default()
    });
    next.nudge.last_nudge_shown_tokens += 1_000;
    next
}

/// The state a chain of `links` folds to.
fn folded(links: &[AgentCompressionUpdate]) -> Option<AgentCompressionState> {
    let mut chain = AgentCompressionChain::default();
    for link in links {
        chain.push(link.clone());
    }
    chain.into_state()
}

/// Writes `states` as a kernel run would, one commit each, from no head.
fn written(states: &[AgentCompressionState]) -> Vec<AgentCompressionUpdate> {
    let mut head = None;
    let mut links = Vec::new();
    for state in states {
        let (link, next) = next_link(head.as_ref(), Some(&Arc::new(state.clone())));
        links.extend(link);
        head = next;
    }
    links
}

#[test]
fn nothing_is_written_without_a_state() {
    let (link, head) = next_link(None, None);
    assert!(link.is_none() && head.is_none());
}

#[test]
fn the_first_link_is_a_snapshot_written_with_one_direction_of_refs() {
    let state = grown(&AgentCompressionState::default(), 6);
    let links = written(std::slice::from_ref(&state));
    let [AgentCompressionUpdate::Snapshot(snapshot)] = links.as_slice() else {
        panic!("one snapshot: {links:?}");
    };
    assert!(snapshot.message_refs.by_ref.is_empty());
    assert_eq!(folded(&links), Some(state));
}

#[test]
fn many_steps_fold_back_to_the_exact_state() {
    let mut states = vec![grown(&AgentCompressionState::default(), 200)];
    for _ in 0..60 {
        states.push(grown(states.last().unwrap(), 10));
    }
    let links = written(&states);
    assert!(matches!(links[1], AgentCompressionUpdate::Delta(_)));
    let snapshots = links
        .iter()
        .filter(|link| matches!(link, AgentCompressionUpdate::Snapshot(_)))
        .count();
    assert!(snapshots > 1 && snapshots < 10, "{snapshots} snapshots");
    assert_eq!(folded(&links).as_ref(), states.last());
}

#[test]
fn a_delta_carries_only_what_changed() {
    let before = grown(&AgentCompressionState::default(), 10);
    let after = grown(&before, 10);
    let delta = diff(&before, &after, 1).unwrap();
    assert_eq!(delta.refs.len(), 10);
    assert_eq!(delta.blocks.len(), 1, "the new block");
    assert_eq!(delta.survived.len(), 1, "the old block only aged");
    assert_eq!(delta.token_snapshot.len(), 7);
    assert_eq!(delta.ts_stamps.len(), 10, "the new messages' stamps");
    assert!(delta.rest.is_some(), "the nudge moved");
}

#[test]
fn an_unchanged_state_writes_nothing() {
    let state = grown(&AgentCompressionState::default(), 4);
    let (_, head) = next_link(None, Some(&Arc::new(state.clone())));
    let (link, head) = next_link(head.as_ref(), Some(&Arc::new(state)));
    assert_eq!(link, None);
    assert!(head.is_some());
}

#[test]
fn a_removed_binding_block_or_stamp_is_written_as_a_snapshot() {
    let before = grown(&AgentCompressionState::default(), 6);
    let mut fewer_refs = grown(&before, 3);
    fewer_refs.message_refs.by_raw.remove("1:0");
    let mut fewer_blocks = grown(&before, 3);
    fewer_blocks.blocks.remove(0);
    let mut fewer_stamps = grown(&before, 3);
    fewer_stamps.ts_stamps.remove("1:0");
    let mut moved_stamp = grown(&before, 3);
    moved_stamp
        .ts_stamps
        .insert("1:1".to_owned(), LocalTime::default());
    for after in [fewer_refs, fewer_blocks, fewer_stamps, moved_stamp] {
        let links = written(&[before.clone(), after.clone()]);
        assert!(matches!(links[1], AgentCompressionUpdate::Snapshot(_)));
        let mut expected = after;
        expected.message_refs.by_ref = super::derived_by_ref(&expected.message_refs);
        assert_eq!(folded(&links), Some(expected));
    }
}

#[test]
fn the_state_s_end_is_written_and_read() {
    let state = grown(&AgentCompressionState::default(), 4);
    let (_, head) = next_link(None, Some(&Arc::new(state)));
    let (link, head) = next_link(head.as_ref(), None);
    assert_eq!(link, Some(AgentCompressionUpdate::Dropped));
    assert!(head.is_none());
}

#[test]
fn a_broken_chain_reads_as_no_state_until_its_next_snapshot() {
    let mut states = vec![grown(&AgentCompressionState::default(), 200)];
    for _ in 0..3 {
        states.push(grown(states.last().unwrap(), 10));
    }
    let links = written(&states);
    assert_eq!(links.len(), 4);

    let missing = [links[0].clone(), links[1].clone(), links[3].clone()];
    assert_eq!(folded(&missing), None);
    let unreadable = [
        links[0].clone(),
        AgentCompressionUpdate::Unreadable,
        links[2].clone(),
    ];
    assert_eq!(folded(&unreadable), None);
    assert_eq!(folded(&links[1..]), None, "no snapshot before the deltas");

    let mut repaired = missing.to_vec();
    repaired.push(written(&states[3..])[0].clone());
    assert_eq!(folded(&repaired).as_ref(), states.last());
}

#[test]
fn a_delta_for_another_state_breaks_the_chain() {
    let base = grown(&AgentCompressionState::default(), 4);
    let mut delta = diff(&base, &grown(&base, 4), 1).unwrap();
    delta.survived.insert("b99".to_owned(), 3);
    let links = [
        written(std::slice::from_ref(&base))[0].clone(),
        AgentCompressionUpdate::Delta(Box::new(delta)),
    ];
    assert_eq!(folded(&links), None);
}

#[test]
fn the_fork_ref_floor_is_not_derived_into_by_ref() {
    let mut state = grown(&AgentCompressionState::default(), 3);
    state
        .message_refs
        .by_raw
        .insert(REF_FLOOR_ID.to_owned(), "m00042".to_owned());
    assert_eq!(folded(&written(std::slice::from_ref(&state))), Some(state));
}
