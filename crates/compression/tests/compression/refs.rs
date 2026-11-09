// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports refs, block-map (spans and created blocks), reversed-ranges
// (orderedRefPair, resolveBlockSpan) and orphan-endpoint-snap (boundaries).

use std::collections::BTreeMap;

use hanafubuki_agent_compression::{
    active_block_spans, assign_refs, format_created_blocks, highest_used_index, index_to_ref,
    ordered_ref_pair, parse_boundary, ref_to_index, resolve_block_span, resolve_boundaries, Block,
    BlockSpan, BoundaryErrorKind, BoundaryKind, Endpoint, Message, RefMap, State, BLOCKED_REF,
};

use crate::fixtures::{apply, assistant, config, range, state_for, user};

fn text(id: &str) -> Message {
    user(id, id)
}

fn map(pairs: &[(&str, &str)]) -> RefMap {
    let mut refs = RefMap::default();
    for (raw, r#ref) in pairs {
        refs.by_raw.insert((*raw).to_owned(), (*r#ref).to_owned());
        if *r#ref != BLOCKED_REF {
            refs.by_ref.insert((*r#ref).to_owned(), (*raw).to_owned());
        }
    }
    refs
}

fn by_raw(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    map(pairs).by_raw
}

#[test]
fn index_to_ref_pads_to_five_digits_and_widens_beyond() {
    assert_eq!(index_to_ref(1).as_deref(), Some("m00001"));
    assert_eq!(index_to_ref(42).as_deref(), Some("m00042"));
    assert_eq!(index_to_ref(99_999).as_deref(), Some("m99999"));
    assert_eq!(index_to_ref(100_000).as_deref(), Some("m100000"));
    assert_eq!(index_to_ref(123_456).as_deref(), Some("m123456"));
    assert_eq!(index_to_ref(9_999_999).as_deref(), Some("m9999999"));
}

#[test]
fn index_to_ref_rejects_out_of_range_indices() {
    assert_eq!(index_to_ref(0), None);
    assert_eq!(index_to_ref(10_000_000), None);
}

#[test]
fn ref_to_index_parses_and_normalizes() {
    assert_eq!(ref_to_index("m00001"), Some(1));
    assert_eq!(ref_to_index("m1"), Some(1));
    assert_eq!(ref_to_index("M0042"), Some(42));
    assert_eq!(ref_to_index("BLOCKED"), None);
    assert_eq!(ref_to_index("b3"), None);
    assert_eq!(ref_to_index("xyz"), None);
}

#[test]
fn ref_to_index_accepts_both_widths_and_enforces_the_cap() {
    assert_eq!(ref_to_index("m100000"), Some(100_000));
    assert_eq!(ref_to_index("m0000001"), Some(1));
    assert_eq!(ref_to_index("m9999999"), Some(9_999_999));
    assert_eq!(ref_to_index("m10000000"), None);
    assert_eq!(ref_to_index("m12345678"), None);
}

#[test]
fn assign_refs_allocates_across_the_old_cap() {
    let existing = map(&[("raw-99999", "m99999")]);
    let assigned = assign_refs(
        &[text("raw-new-1"), text("raw-new-2")],
        &existing,
        99_999,
        |_| false,
    );
    assert_eq!(assigned.map.by_raw["raw-new-1"], "m100000");
    assert_eq!(
        ref_to_index(&assigned.map.by_raw["raw-new-2"]),
        Some(100_001)
    );
    assert_eq!(assigned.next_index, 100_002);
    assert_eq!(assigned.newly_assigned, 2);
}

#[test]
fn assign_refs_numbers_new_messages_in_order() {
    let assigned = assign_refs(
        &[text("a"), text("b"), text("c")],
        &RefMap::default(),
        1,
        |_| false,
    );
    assert_eq!(assigned.newly_assigned, 3);
    assert_eq!(assigned.map.by_raw["a"], "m00001");
    assert_eq!(assigned.map.by_raw["b"], "m00002");
    assert_eq!(assigned.map.by_raw["c"], "m00003");
    assert_eq!(assigned.map.by_ref["m00002"], "b");
}

#[test]
fn assign_refs_keeps_existing_refs_and_continues_numbering() {
    let existing = map(&[("a", "m00001")]);
    let assigned = assign_refs(&[text("a"), text("b")], &existing, 5, |_| false);
    assert_eq!(assigned.newly_assigned, 1);
    assert_eq!(assigned.map.by_raw["a"], "m00001");
    assert_eq!(assigned.map.by_raw["b"], "m00005");
}

#[test]
fn assign_refs_blocks_protected_messages_without_taking_a_number() {
    let assigned = assign_refs(
        &[text("a"), text("b"), text("c")],
        &RefMap::default(),
        1,
        |m| m.id == "b",
    );
    assert_eq!(assigned.map.by_raw["a"], "m00001");
    assert_eq!(assigned.map.by_raw["b"], BLOCKED_REF);
    assert_eq!(assigned.map.by_raw["c"], "m00002");
    assert_eq!(assigned.newly_assigned, 2);
}

#[test]
fn assign_refs_skips_numbers_already_taken() {
    let existing = map(&[("old", "m00002")]);
    let assigned = assign_refs(&[text("x")], &existing, 1, |_| false);
    assert_eq!(assigned.map.by_raw["x"], "m00001");
    assert_eq!(assigned.next_index, 2);
}

#[test]
fn highest_used_index_is_the_largest_numeric_ref() {
    let refs = map(&[("a", "m00003"), ("b", "m00010"), ("c", BLOCKED_REF)]);
    assert_eq!(highest_used_index(&refs), 10);
}

#[test]
fn parse_boundary_accepts_widened_refs_and_block_refs() {
    let wide = parse_boundary("m100000").expect("a widened ref");
    assert_eq!(
        (wide.kind, wide.numeric_id),
        (BoundaryKind::Message, 100_000)
    );
    assert_eq!(
        parse_boundary("m9999999").map(|b| b.numeric_id),
        Some(9_999_999)
    );
    assert!(parse_boundary("m10000000").is_none());
    assert_eq!(parse_boundary("m00001").map(|b| b.numeric_id), Some(1));
    assert_eq!(parse_boundary("m1").map(|b| b.numeric_id), Some(1));
    assert_eq!(
        parse_boundary("b3").map(|b| b.kind),
        Some(BoundaryKind::Block)
    );
}

#[test]
fn ordered_ref_pair_sorts_reversed_pairs_and_keeps_everything_else() {
    assert_eq!(ordered_ref_pair("m00760", "m00651"), ("m00651", "m00760"));
    assert_eq!(ordered_ref_pair("m00651", "m00760"), ("m00651", "m00760"));
    assert_eq!(ordered_ref_pair("m00010", "m00010"), ("m00010", "m00010"));
    assert_eq!(ordered_ref_pair("BLOCKED", "m00010"), ("BLOCKED", "m00010"));
    assert_eq!(ordered_ref_pair("b1", "b2"), ("b1", "b2"));
}

fn block(block_id: &str, effective: &[&str]) -> Block {
    Block {
        block_id: block_id.to_owned(),
        run_id: "r1".to_owned(),
        summary: "s".to_owned(),
        effective_message_ids: effective.iter().map(|id| (*id).to_owned()).collect(),
        compressed_tokens: 100,
        ..Block::default()
    }
}

fn with_refs(mut block: Block, start: &str, end: &str) -> Block {
    block.start_ref = Some(start.to_owned());
    block.end_ref = Some(end.to_owned());
    block
}

fn span(start: &str, end: &str) -> Option<(String, String)> {
    Some((start.to_owned(), end.to_owned()))
}

#[test]
fn a_block_span_prefers_its_stored_refs() {
    let stored = with_refs(block("b1", &["a", "b"]), "m00010", "m00020");
    assert_eq!(
        resolve_block_span(&stored, &BTreeMap::new()),
        span("m00010", "m00020")
    );
}

#[test]
fn a_block_span_falls_back_to_its_coverage_sorted_numerically() {
    let refs = by_raw(&[("a", "m00044"), ("b", "m00001"), ("c", "m00030")]);
    assert_eq!(
        resolve_block_span(&block("b1", &["a", "b", "c"]), &refs),
        span("m00001", "m00044")
    );
    let refs = by_raw(&[("a", "m00009"), ("b", "m000100")]);
    assert_eq!(
        resolve_block_span(&block("b1", &["a", "b"]), &refs),
        span("m00009", "m000100")
    );
}

#[test]
fn a_block_span_is_none_when_nothing_resolves() {
    assert_eq!(
        resolve_block_span(&block("b1", &["gone"]), &BTreeMap::new()),
        None
    );
}

#[test]
fn a_block_span_ignores_stored_block_and_mixed_refs() {
    let refs = by_raw(&[("a", "m00001"), ("b", "m00020")]);
    let mut distilled = with_refs(block("b3", &["a", "b"]), "b1", "b2");
    distilled.tier = 2;
    assert_eq!(
        resolve_block_span(&distilled, &refs),
        span("m00001", "m00020")
    );
    let mixed = with_refs(block("b3", &["a", "b"]), "b1", "m00020");
    assert_eq!(resolve_block_span(&mixed, &refs), span("m00001", "m00020"));
}

#[test]
fn a_stored_reversed_span_is_canonicalized() {
    let reversed = with_refs(block("b1", &[]), "m00760", "m00651");
    assert_eq!(
        resolve_block_span(&reversed, &BTreeMap::new()),
        span("m00651", "m00760")
    );
    let state = State {
        blocks: vec![reversed.clone()],
        ..State::default()
    };
    assert_eq!(
        active_block_spans(&state),
        [BlockSpan {
            block_id: "b1".to_owned(),
            tier: 1,
            start_ref: "m00651".to_owned(),
            end_ref: "m00760".to_owned(),
        }]
    );
    assert_eq!(
        format_created_blocks(&state, &[reversed]),
        "blocks: b1=m00651–m00760"
    );
}

#[test]
fn a_tier_distilled_block_reports_a_message_ref_span() {
    let messages: Vec<Message> = (1..=20)
        .map(|i| user(&format!("id{i}"), &format!("message {i}")))
        .collect();
    let state = state_for(&messages);
    let state = apply(
        &[
            range("m00001", "m00010", "first ten"),
            range("m00011", "m00020", "next ten"),
        ],
        &messages,
        &state,
        &config(),
    )
    .state;
    let state = apply(
        &[range("b1", "b2", "distilled")],
        &messages,
        &state,
        &config(),
    )
    .state;
    let distilled = state.blocks.last().expect("a block");
    assert_eq!(distilled.tier, 2);
    assert_eq!(
        (distilled.start_ref.as_deref(), distilled.end_ref.as_deref()),
        (Some("b1"), Some("b2"))
    );
    assert_eq!(
        active_block_spans(&state),
        [BlockSpan {
            block_id: distilled.block_id.clone(),
            tier: 2,
            start_ref: "m00001".to_owned(),
            end_ref: "m00020".to_owned(),
        }]
    );
}

#[test]
fn active_block_spans_skip_inactive_and_unresolvable_blocks() {
    let mut state = state_for(&[text("a"), text("b")]);
    let mut inactive = with_refs(block("b2", &[]), "m00002", "m00002");
    inactive.active = false;
    let mut vanished = block("b3", &["vanished"]);
    vanished.tier = 2;
    state.blocks = vec![
        with_refs(block("b1", &["a"]), "m00001", "m00001"),
        inactive,
        vanished,
    ];
    let spans: Vec<String> = active_block_spans(&state)
        .into_iter()
        .map(|span| span.block_id)
        .collect();
    assert_eq!(spans, ["b1"]);
}

#[test]
fn created_blocks_are_formatted_by_span_or_bare_id() {
    let state = State::default();
    let blocks = [
        with_refs(block("b3", &[]), "m00044", "m00097"),
        with_refs(block("b4", &[]), "m00103", "m00123"),
    ];
    assert_eq!(
        format_created_blocks(&state, &blocks),
        "blocks: b3=m00044–m00097, b4=m00103–m00123"
    );
    assert_eq!(
        format_created_blocks(&state, &[block("b5", &["gone"])]),
        "blocks: b5"
    );
    assert_eq!(format_created_blocks(&state, &[]), "");
}

fn visible(id: &str) -> Message {
    assistant(id, id)
}

fn orphan_state(pairs: &[(&str, &str)], hidden: &[&str]) -> State {
    State {
        message_refs: map(pairs),
        hidden_orphan_refs: hidden.iter().map(|r| (*r).to_owned()).collect(),
        ..State::default()
    }
}

#[test]
fn a_hidden_orphan_start_snaps_forward() {
    let messages: Vec<Message> = ["v1", "v2", "v3", "v6", "v7", "v8"].map(visible).to_vec();
    let state = orphan_state(
        &[
            ("v1", "m00001"),
            ("v2", "m00002"),
            ("v3", "m00003"),
            ("orphanCall", "m00005"),
            ("v6", "m00006"),
            ("v7", "m00007"),
            ("v8", "m00008"),
        ],
        &["m00005"],
    );
    let resolved = resolve_boundaries("m00005", "m00008", &messages, &state).expect("resolves");
    assert_eq!((resolved.start_index, resolved.end_index), (3, 5));
    assert_eq!(resolved.snapped_boundaries.len(), 1);
}

#[test]
fn a_hidden_orphan_end_snaps_backward() {
    let messages: Vec<Message> = ["v1", "v2", "v3", "v6"].map(visible).to_vec();
    let state = orphan_state(
        &[
            ("v1", "m00001"),
            ("v2", "m00002"),
            ("v3", "m00003"),
            ("orphanCall", "m00005"),
            ("v6", "m00006"),
        ],
        &["m00005"],
    );
    let resolved = resolve_boundaries("m00001", "m00005", &messages, &state).expect("resolves");
    assert_eq!(resolved.end_index, 2);
    assert_eq!(resolved.snapped_boundaries.len(), 1);
}

#[test]
fn a_dangling_ref_outside_the_hidden_orphans_stays_consumed() {
    let messages: Vec<Message> = ["v1", "v2", "v6"].map(visible).to_vec();
    let state = orphan_state(
        &[
            ("v1", "m00001"),
            ("v2", "m00002"),
            ("driftedGone", "m00005"),
            ("v6", "m00006"),
        ],
        &[],
    );
    let error = resolve_boundaries("m00005", "m00006", &messages, &state).expect_err("dangling");
    assert_eq!(error.kind, BoundaryErrorKind::Consumed);
    assert_eq!(error.endpoint, Some(Endpoint::Start));
}

#[test]
fn a_hidden_orphan_without_a_neighbour_gives_an_accurate_error() {
    let messages: Vec<Message> = ["v1", "v2"].map(visible).to_vec();
    let state = orphan_state(
        &[("v1", "m00001"), ("v2", "m00002"), ("orphanCall", "m00003")],
        &["m00003"],
    );
    let error =
        resolve_boundaries("m00003", "m00002", &messages, &state).expect_err("no neighbour");
    assert_eq!(error.kind, BoundaryErrorKind::Consumed);
    assert!(
        error.message.contains("no adjacent visible message"),
        "{}",
        error.message
    );
}

#[test]
fn a_range_whose_endpoint_is_a_hidden_orphan_compresses() {
    let names = [
        "u1", "a1", "b1", "c1", "d1", "e1", "f1", "g1", "h1", "i1", "j1", "k1",
    ];
    let body = "lorem ipsum dolor sit amet ".repeat(20);
    let messages: Vec<Message> = names.iter().map(|id| assistant(id, &body)).collect();
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut number = 1;
    for id in names {
        if number == 5 {
            number = 6;
        }
        pairs.push((id.to_owned(), format!("m{number:05}")));
        number += 1;
    }
    pairs.push(("orphanCall".to_owned(), "m00005".to_owned()));
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(raw, r#ref)| (raw.as_str(), r#ref.as_str()))
        .collect();
    let state = orphan_state(&pairs, &["m00005"]);

    let outcome = apply(
        &[range("m00005", "m00013", "recap of the visible span")],
        &messages,
        &state,
        &config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|w| w.contains("snapped")),
        "{:?}",
        outcome.result.warnings
    );
}
