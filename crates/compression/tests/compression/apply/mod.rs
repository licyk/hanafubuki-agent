// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).

// Ports `compress.test.ts`; `batch` ports the rest of the apply group.

mod batch;

use hanafubuki_agent_compression::{
    prune, resolve_boundaries, Block, BoundaryErrorKind, Config, Endpoint, Message, State,
};

use crate::fixtures::{apply, assign, config, ids, range, sorted, state_for, user, with_topic};

const LONG_TEXT_LEN: usize = 6000;
const VALID_SUMMARY: &str = "This is a meaningful summary that captures the key information of the compressed range including paths and decisions.";

fn gated(min_compress_range: usize) -> Config {
    let mut config = config();
    config.compress.min_compress_range = min_compress_range;
    config
}

/// The kernel's defaults for a 200K window with the recent zone off.
fn defaults() -> Config {
    let mut config = Config::new(200_000);
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

fn long(id: &str) -> Message {
    user(id, &"x".repeat(LONG_TEXT_LEN))
}

fn block(
    id: &str,
    tier: u8,
    direct: &[&str],
    effective: &[&str],
    consumed: &[&str],
    active: bool,
) -> Block {
    let strings = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect();
    Block {
        block_id: id.to_owned(),
        run_id: "r1".to_owned(),
        tier,
        topic: Some("t".to_owned()),
        summary: format!("summary of {id}"),
        direct_message_ids: strings(direct),
        effective_message_ids: strings(effective),
        direct_block_ids: strings(consumed),
        active,
        ..Block::default()
    }
}

fn five() -> Vec<Message> {
    vec![
        user("u", "the task"),
        user("a", "alpha"),
        user("b", "beta"),
        user("c", "gamma"),
        user("d", "delta"),
    ]
}

/// `five()` with `m00002..m00003` folded into `b1`, and its pruned view.
fn folded_five(messages: &[Message]) -> (State, Vec<Message>) {
    let state = state_for(messages);
    let after = apply(
        &[range("m00002", "m00003", "intro recap")],
        messages,
        &state,
        &config(),
    )
    .state;
    let pruned = prune(messages, &after);
    (after, pruned)
}

// compress.test.ts

#[test]
fn apply_compression_creates_a_t1_block_covering_the_resolved_range() {
    let messages = vec![
        user("a", "alpha"),
        user("b", "beta"),
        user("c", "gamma"),
        user("d", "delta"),
    ];
    let state = state_for(&messages);
    let outcome = apply(
        &[with_topic(
            range("m00001", "m00002", "a and b summarized"),
            "intro",
        )],
        &messages,
        &state,
        &config(),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
    assert_eq!(outcome.state.blocks.len(), 1);
    let block = &outcome.state.blocks[0];
    assert_eq!(block.tier, 1);
    assert!(block.active);
    assert_eq!(block.topic.as_deref(), Some("intro"));
    assert_eq!(sorted(block.effective_message_ids.clone()), ["a", "b"]);
    assert_eq!(sorted(block.direct_message_ids.clone()), ["a", "b"]);
    assert!(outcome.result.tokens_compressed > 0);
}

#[test]
fn prune_after_apply_compression_removes_covered_messages_and_injects_summary() {
    let messages = five();
    let (after, pruned) = folded_five(&messages);
    assert_eq!(after.blocks.len(), 1);
    assert_eq!(ids(&pruned), ["u", "acp_summary_b1", "c", "d"]);
    assert!(pruned[1].text.contains("intro recap"));
}

#[test]
fn resolve_boundaries_normalizes_reversed_refs_and_records_a_transparency_note() {
    let messages = vec![user("a", "x"), user("b", "y"), user("c", "z")];
    let state = state_for(&messages);
    let reversed = resolve_boundaries("m00003", "m00001", &messages, &state).unwrap();
    assert_eq!((reversed.start_index, reversed.end_index), (0, 2));
    assert_eq!(reversed.message_ids, ["a", "b", "c"]);
    assert_eq!(
        reversed.reversed_note.as_deref(),
        Some("note: refs were given reversed (m00003→m00001), normalized to m00001..m00003")
    );
    let forward = resolve_boundaries("m00001", "m00003", &messages, &state).unwrap();
    assert_eq!(forward.reversed_note, None);
}

#[test]
fn gate_verdict_carries_the_reversal_note_instead_of_hiding_it() {
    let messages = vec![user("a", "x"), user("b", "y"), user("c", "z")];
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00003", "m00001", "reversed")],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.state.blocks.is_empty());
    assert_eq!(outcome.result.errors.len(), 1);
    let error = &outcome.result.errors[0];
    assert!(
        error.contains("too small (3 chars across 1 range(s), min 5000)"),
        "{error}"
    );
    assert!(error
        .contains("note: refs were given reversed (m00003→m00001), normalized to m00001..m00003"));
    assert!(outcome.result.notes.is_empty());
}

#[test]
fn successful_compression_of_a_reversed_range_reports_the_rewrite_in_notes() {
    let big = "w".repeat(1000);
    let messages: Vec<Message> = ["a", "b", "c", "d", "e", "f"]
        .iter()
        .map(|id| user(id, &big))
        .collect();
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00006", "m00001", "big reversed")],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(
        outcome.result.notes,
        ["note: refs were given reversed (m00006→m00001), normalized to m00001..m00006"]
    );
}

#[test]
fn mixed_batch_appends_the_reversal_note_to_the_too_small_verdict() {
    let messages: Vec<Message> = [("a", "x"), ("b", "y"), ("c", "z"), ("d", "w"), ("e", "v")]
        .iter()
        .map(|(id, text)| user(id, text))
        .collect();
    let state = state_for(&messages);
    let outcome = apply(
        &[
            range("m00001", "m00002", "tiny ok"),
            range("m00005", "m00004", "reversed pair"),
        ],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 1);
    let error = &outcome.result.errors[0];
    assert!(
        error.contains("too small (4 chars across 2 range(s), min 5000)"),
        "{error}"
    );
    assert!(error
        .contains("note: refs were given reversed (m00005→m00004), normalized to m00004..m00005"));
}

#[test]
fn all_invalid_batch_reports_per_range_errors_without_fabricating_a_size_verdict() {
    let messages = vec![user("a", "x"), user("b", "y"), user("c", "z")];
    let state = state_for(&messages);
    let outcome = apply(
        &[range("foo", "bar", "bad")],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(outcome.result.errors[0].contains("Invalid boundary ref(s)"));
    assert!(!outcome
        .result
        .errors
        .join("\n")
        .to_lowercase()
        .contains("too small"));
}

#[test]
fn block_boundary_compression_produces_t2_and_consumes_matching_t1_blocks() {
    let messages: Vec<Message> = [
        ("a", "alpha"),
        ("b", "beta"),
        ("c", "gamma"),
        ("d", "delta"),
        ("e", "epsilon"),
    ]
    .iter()
    .map(|(id, text)| user(id, text))
    .collect();
    let state = state_for(&messages);
    let t1 = apply(
        &[range("m00001", "m00002", "t1 block a-b")],
        &messages,
        &state,
        &config(),
    );
    let t2 = apply(
        &[range("b1", "b1", "t2 distillation of b1")],
        &messages,
        &t1.state,
        &config(),
    );
    let block = &t2.state.blocks[1];
    assert_eq!(block.tier, 2);
    assert_eq!(block.block_id, "b2");
    assert!(!t2.state.blocks[0].active);
    assert_eq!(sorted(block.effective_message_ids.clone()), ["a", "b"]);
}

#[test]
fn apply_compression_reports_error_for_unknown_boundary_ref() {
    let messages = vec![user("a", "x")];
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00099", "m00100", "nope")],
        &messages,
        &state,
        &config(),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 1);
    // "unknown ref (valid format, never allocated) names the ref and suggests acp_status"
    let error = &outcome.result.errors[0];
    assert!(
        error.contains("startId=\"m00099\" is above this session's highest allocated ref (m00001)"),
        "{error}"
    );
    assert!(error.contains("run acp_status"));
}

#[test]
fn batch_compress_rejects_the_whole_batch_when_a_summary_fails_length_validation() {
    let messages: Vec<Message> = [
        ("a", "alpha"),
        ("b", "beta"),
        ("c", "gamma"),
        ("d", "delta"),
    ]
    .iter()
    .map(|(id, text)| user(id, text))
    .collect();
    let state = state_for(&messages);
    let mut config = config();
    config.compress.min_summary_length = 50;
    let outcome = apply(
        &[
            with_topic(range("m00001", "m00002", &"x".repeat(60)), "ok"),
            with_topic(range("m00003", "m00004", &"y".repeat(22)), "short"),
        ],
        &messages,
        &state,
        &config,
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.state.blocks.is_empty());
    assert_eq!(outcome.result.errors.len(), 1);
    let error = &outcome.result.errors[0];
    assert!(error.contains("Batch rejected before folding any range"));
    assert!(
        error.contains("range m00003..m00004: Summary too short (22 chars, min 50)"),
        "{error}"
    );
}

#[test]
fn retrying_a_consumed_range_reports_already_compressed_guidance_not_too_small() {
    let messages = five();
    let (after, pruned) = folded_five(&messages);
    let retry = apply(
        &[range("m00002", "m00003", "intro recap")],
        &pruned,
        &after,
        &gated(5000),
    );
    assert_eq!(retry.result.blocks_created, 0);
    assert!(retry.result.errors.is_empty());
    assert!(retry.result.noop);
    let warning = &retry.result.warnings[0];
    assert!(warning.contains("already compressed"));
    assert!(warning.contains("already summarized in active block(s) b1"));
    assert!(warning.contains("do not resend these ranges"));
    assert!(!warning.contains("retry with startId/endId set to active block IDs"));
    assert!(!warning.contains("Total compressible content too small"));
}

#[test]
fn consumed_plus_fresh_but_small_range_is_not_misreported_as_too_small() {
    let messages = five();
    let (after, pruned) = folded_five(&messages);
    let retry = apply(
        &[
            range("m00002", "m00003", "intro recap"),
            range("m00004", "m00005", "c and d"),
        ],
        &pruned,
        &after,
        &gated(5000),
    );
    assert_eq!(retry.result.blocks_created, 0);
    assert_eq!(retry.result.errors.len(), 1);
    let error = &retry.result.errors[0];
    assert!(error.contains("already compressed"));
    assert!(error.contains("already summarized in active block(s) b1"));
    assert!(
        error.contains(
            "[diagnostics: session highest allocated ref=m00005 (the ref space is sparse"
        ),
        "{error}"
    );
    assert!(error.contains(
        "), unknown ranges in request=0/2, session history=1 compression(s), 1 block(s)]"
    ));
    assert!(!error.to_lowercase().contains("renumber"));
    assert!(error.contains("un acp_status"));
    assert!(!error.contains("Combine more messages"));
}

#[test]
fn all_unknown_batch_reports_stale_refs_instead_of_too_small() {
    let messages = five();
    let state = state_for(&messages);
    let outcome = apply(
        &[
            range("m00050", "m00060", "stale A"),
            range("m00070", "m00080", "stale B"),
        ],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 3);
    let error = &outcome.result.errors[0];
    assert!(error.contains("None of the 2 requested range(s) resolved"));
    assert!(error.contains("no compress reassigns them"));
    assert!(
        error.contains("failing endpoints: startId=m00050, startId=m00070"),
        "{error}"
    );
    assert!(!error.contains("cannot come from an earlier compress in this session"));
    assert!(error.contains("stale artifact from an earlier generation"));
    assert!(error.contains("the ref space is sparse"));
    assert!(error.contains("Do not probe for valid numbers"));
    assert!(error
        .contains("[diagnostics: session highest allocated ref=m00005 (the ref space is sparse"));
    assert!(error.contains(
        "), unknown ranges in request=2/2, session history=0 compression(s), 0 block(s)]"
    ));
    assert!(error.contains("Run acp_status"));
    assert!(!error.contains("too small"));
    assert!(outcome.result.errors[1]
        .contains("startId=\"m00050\" is above this session's highest allocated ref (m00005)"));
    assert!(outcome.result.errors[2]
        .contains("startId=\"m00070\" is above this session's highest allocated ref (m00005)"));
}

#[test]
fn consumed_plus_unknown_ranges_keep_the_already_compressed_message() {
    let messages = five();
    let (after, pruned) = folded_five(&messages);
    let retry = apply(
        &[
            range("m00002", "m00003", "intro recap"),
            range("m00050", "m00060", "stale"),
        ],
        &pruned,
        &after,
        &gated(5000),
    );
    assert_eq!(retry.result.blocks_created, 0);
    assert!(retry.result.errors[0].contains("already compressed"));
    assert!(!retry.result.errors[0].contains("None of the 2 requested range(s) resolved"));
    let stale = retry
        .result
        .errors
        .iter()
        .find(|error| error.starts_with("range m00050..m00060"))
        .expect("a per-range error for the stale range");
    assert!(stale.contains("startId=\"m00050\" is above this session's highest allocated ref"));
}

#[test]
fn first_compress_of_a_fresh_session_with_foreign_refs_reports_a_new_generation() {
    let messages = vec![user("u", "the task"), user("a", "alpha"), user("b", "beta")];
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00050", "m00060", "foreign")],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 2);
    let error = &outcome.result.errors[0];
    assert!(error.contains("None of the 1 requested range(s) resolved"));
    assert!(error.contains("failing endpoints: startId=m00050"));
    assert!(error.contains("switching model or upstream"));
    assert!(error.contains("native-compaction rebase"));
    assert!(error
        .contains("[diagnostics: session highest allocated ref=m00003 (the ref space is sparse"));
    assert!(error.contains(
        "), unknown ranges in request=1/1, session history=0 compression(s), 0 block(s)]"
    ));
    assert!(outcome.result.errors[1]
        .contains("startId=\"m00050\" is above this session's highest allocated ref (m00003)"));
}

#[test]
fn refs_from_before_a_native_compaction_rebase_report_a_new_generation_with_zeroed_history() {
    let old = five();
    let state = state_for(&old);
    let after_one = apply(&[range("m00002", "m00003", "s1")], &old, &state, &config()).state;
    let after_two = apply(
        &[range("m00004", "m00005", "s2")],
        &old,
        &after_one,
        &config(),
    )
    .state;
    assert_eq!(after_two.stats.compression_count, 2);

    let fresh = vec![user("u2", "new task"), user("e", "epsilon")];
    let rebased = state_for(&fresh);
    let outcome = apply(
        &[range("m00004", "m00005", "old refs")],
        &fresh,
        &rebased,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    let error = &outcome.result.errors[0];
    assert!(error.contains("None of the 1 requested range(s) resolved"));
    assert!(error.contains("native-compaction rebase"));
    assert!(error.contains("refs restart at m00001"));
    assert!(error
        .contains("[diagnostics: session highest allocated ref=m00002 (the ref space is sparse"));
    assert!(error.contains(
        "), unknown ranges in request=1/1, session history=0 compression(s), 0 block(s)]"
    ));
}

#[test]
fn drifted_message_content_dangles_the_old_ref_reported_as_unanchorable() {
    let messages = five();
    let mut state = state_for(&messages);
    let mut drifted = five();
    drifted[4] = user("d2", "delta (edited)");
    assign(&drifted, &mut state);
    let outcome = apply(
        &[range("m00004", "m00005", "c and d")],
        &drifted,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    let error = &outcome.result.errors[0];
    assert!(error.contains("cannot be anchored"));
    assert!(error.contains("no active block covers them"));
    assert!(error.contains("recorded as DEAD"));
    assert!(error.contains("Do not retry this range in any form"));
    assert_eq!(outcome.state.dead_refs, ["m00005"]);
    assert!(error
        .contains("[diagnostics: session highest allocated ref=m00006 (the ref space is sparse"));
    assert!(error.contains(
        "), unknown ranges in request=0/1, session history=0 compression(s), 0 block(s)]"
    ));
    assert!(!error.contains("already compressed"));
}

#[test]
fn fresh_small_content_without_consumed_ranges_keeps_the_too_small_message() {
    let messages = vec![user("a", "alpha"), user("b", "beta")];
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00001", "m00002", "a and b")],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.errors[0]
        .starts_with("Total compressible content too small (9 chars across 1 range(s), min 5000)"));
}

#[test]
fn consumed_plus_fresh_content_above_threshold_proceeds_with_a_warning() {
    let mut messages = five();
    messages[3] = user("c", &"z".repeat(6000));
    let (after, pruned) = folded_five(&messages);
    let retry = apply(
        &[
            range("m00002", "m00003", "intro recap"),
            range("m00004", "m00005", "big block"),
        ],
        &pruned,
        &after,
        &gated(5000),
    );
    assert_eq!(retry.result.blocks_created, 1);
    assert!(retry.result.errors.is_empty(), "{:?}", retry.result.errors);
    assert!(
        retry
            .result
            .warnings
            .iter()
            .any(|w| w.contains("Skipped range (m00002..m00003) — already compressed")),
        "{:?}",
        retry.result.warnings
    );
}

#[test]
fn empty_summary_is_attributed_to_its_range() {
    let messages = vec![user("a", "alpha"), user("b", "beta")];
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00001", "m00002", "")],
        &messages,
        &state,
        &config(),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(outcome.result.errors[0].contains("range m00001..m00002: Summary is empty"));
}

#[test]
fn invalid_refs_are_reported_per_range_without_failing_the_batch() {
    let messages = vec![user("a", "alpha"), user("b", "beta")];
    let state = state_for(&messages);
    let outcome = apply(
        &[
            range("m10000000", "m00002", "bad ref"),
            range("m00001", "m00002", "good summary"),
        ],
        &messages,
        &state,
        &config(),
    );
    assert_eq!(
        outcome.result.blocks_created, 1,
        "the valid range still compresses"
    );
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(outcome.result.errors[0].starts_with("range m10000000..m00002: Invalid boundary ref"));
}

#[test]
fn consumed_ranges_warn_and_skip_when_min_compress_range_is_zero() {
    let messages = vec![
        user("u", "the task"),
        user("a", "alpha"),
        user("b", "beta"),
        user("c", "gamma"),
    ];
    let (after, pruned) = folded_five(&messages);
    let retry = apply(
        &[range("m00002", "m00003", "intro recap")],
        &pruned,
        &after,
        &config(),
    );
    assert_eq!(retry.result.blocks_created, 0);
    assert!(retry.result.errors.is_empty());
    assert!(retry
        .result
        .warnings
        .iter()
        .any(|w| w.contains("Skipped range (m00002..m00003) — already compressed")));
}

#[test]
fn resolve_boundaries_fails_with_a_typed_error_naming_kind_and_endpoint() {
    let messages = vec![
        user("u", "the task"),
        user("a", "alpha"),
        user("b", "beta"),
        user("c", "gamma"),
    ];
    let state = state_for(&messages);
    let unknown = resolve_boundaries("m00099", "m00001", &messages, &state).unwrap_err();
    assert_eq!(unknown.kind, BoundaryErrorKind::Unknown);
    assert_eq!(unknown.endpoint, Some(Endpoint::Start));

    let after = apply(
        &[range("m00002", "m00003", "a and b")],
        &messages,
        &state,
        &config(),
    )
    .state;
    let pruned = prune(&messages, &after);
    let consumed = resolve_boundaries("m00002", "m00003", &pruned, &after).unwrap_err();
    assert_eq!(consumed.kind, BoundaryErrorKind::Consumed);
    assert_eq!(consumed.endpoint, Some(Endpoint::Start));
}

fn twelve() -> Vec<Message> {
    let mut messages = vec![user("a", "old one"), user("b", "old two")];
    for id in ["c", "d", "e", "f", "g", "h", "i", "j"] {
        messages.push(user(id, &format!("raw {id}")));
    }
    messages.push(user("k", "recent one"));
    messages.push(user("l", "recent two"));
    messages
}

#[test]
fn consumed_block_anchor_snaps_to_the_active_owning_block() {
    let messages = twelve();
    let mut state = state_for(&messages);
    state
        .blocks
        .push(block("b2", 1, &["a"], &["a", "b"], &[], false));
    state
        .blocks
        .push(block("b50", 2, &[], &["a", "b"], &["b2"], true));
    state
        .blocks
        .push(block("b110", 1, &["k"], &["k", "l"], &[], true));
    state.next_block_id = 111;
    let outcome = apply(
        &[with_topic(range("b2", "b110", "distilled span"), "t")],
        &messages,
        &state,
        &config(),
    );
    assert_eq!(outcome.result.blocks_created, 1, "{:?}", outcome.result);
    assert!(outcome.result.errors.is_empty());
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|w| w.contains("startId=\"b2\" was consumed by a higher-tier block")),
        "{:?}",
        outcome.result.warnings
    );
    let created = outcome
        .state
        .block("b111")
        .expect("new block allocated after b110");
    assert_eq!(created.tier, 2);
    assert_eq!(created.direct_block_ids, ["b110"]);
    assert!(!outcome.state.block("b110").unwrap().active);
    assert!(outcome.state.block("b50").unwrap().active);
}

#[test]
fn consumed_message_anchor_snaps_to_the_active_block_covering_it() {
    let full = twelve();
    let mut state = state_for(&full);
    state
        .blocks
        .push(block("b2", 1, &["a", "b"], &["a", "b"], &[], false));
    state
        .blocks
        .push(block("b50", 2, &["c"], &["a", "b", "c"], &["b2"], true));
    state
        .blocks
        .push(block("b110", 1, &["k"], &["k", "l"], &[], true));
    let visible = &full[2..];
    let outcome = apply(
        &[with_topic(range("m00001", "b110", "distilled span"), "t")],
        visible,
        &state,
        &config(),
    );
    assert_eq!(outcome.result.blocks_created, 1, "{:?}", outcome.result);
    assert!(outcome.result.errors.is_empty());
    assert!(outcome
        .result
        .warnings
        .iter()
        .any(|w| w.contains("startId=\"m00001\" refers to a message already compressed")));
}

#[test]
fn gate_error_names_the_covering_block_when_anchors_stay_consumed() {
    let messages = vec![user("a", "x"), user("b", "y"), user("k", "z")];
    let mut state = state_for(&messages);
    state
        .blocks
        .push(block("b2", 1, &["a"], &["a", "b"], &[], false));
    state
        .blocks
        .push(block("b110", 1, &["k"], &["k"], &[], true));
    let outcome = apply(
        &[with_topic(range("b2", "b110", "distilled span"), "t")],
        &messages,
        &state,
        &gated(5000),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.noop);
    let warning = &outcome.result.warnings[0];
    assert!(
        warning.contains("Requested range(s) already compressed (e.g. b2..b110)"),
        "{warning}"
    );
    assert!(warning.contains(
        "its content is already summarized in active block(s) b110 — use search_context or decompress b110 if you need details from it"
    ));
    assert!(warning.contains("do not resend these ranges"));
    assert!(!warning.contains("retry with startId/endId set to active block IDs"));
}
