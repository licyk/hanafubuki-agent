// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `issue4484-merge-coverage.test.ts`, `issue5789-tier2-merge-ladder.test.ts`
// and `issue5919-shredded-merge.test.ts`; the kernel's MCP-prefixed
// protected rule tool is the bare `acp_rule` here.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    compute_integrity_withdrawals, ApplyOutcome, Block, Config, Message, Role, State, BLOCKED_REF,
};

use crate::fixtures::{apply, range};

fn summary() -> String {
    "Synthetic summary for this compression test. ".repeat(4)
}

fn long(repeat: usize) -> String {
    "Synthetic historical content, safe for public reproduction. ".repeat(repeat)
}

fn merge_config() -> Config {
    let mut config = Config::new(255_000);
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

fn user(id: &str, text: &str) -> Message {
    Message::text(id, Role::User, text)
}

fn assistant(id: &str, text: &str) -> Message {
    Message::text(id, Role::Assistant, text)
}

fn reasoning(id: &str, text: &str) -> Message {
    Message::reasoning(id, text)
}

fn call(id: &str, tool: &str, call_id: &str, arguments: &str) -> Message {
    Message::tool_call(id, call_id, tool, arguments)
}

fn result(id: &str, tool: &str, call_id: &str, output: &str) -> Message {
    Message::tool_result(id, call_id, tool, output)
}

/// Refs in array order; the messages in `blocked` take no number.
fn refs_skipping(messages: &[Message], blocked: &[&str]) -> State {
    let mut state = State::default();
    let mut next = 1;
    for message in messages {
        let r#ref = if blocked.contains(&message.id.as_str()) {
            BLOCKED_REF.to_owned()
        } else {
            let r#ref = format!("m{next:05}");
            next += 1;
            state
                .message_refs
                .by_ref
                .insert(r#ref.clone(), message.id.clone());
            r#ref
        };
        state.message_refs.by_raw.insert(message.id.clone(), r#ref);
    }
    state
}

/// Folds the first two messages into `b1`, then pushes hand-built blocks
/// (`id`, tier, covered ids), the shape a defective older kernel left.
fn seeded(messages: &[Message], blocked: &[&str], pushed: &[(&str, u8, &[&str])]) -> State {
    let state = refs_skipping(messages, blocked);
    let first = &state.message_refs.by_raw;
    let seed = apply(
        &[range(
            &first[&messages[0].id],
            &first[&messages[1].id],
            &summary(),
        )],
        messages,
        &state,
        &merge_config(),
    );
    assert_eq!(
        seed.result.blocks_created, 1,
        "fixture seed: {:?}",
        seed.result
    );
    assert!(seed.result.errors.is_empty());
    let mut state = seed.state;
    let base = state.blocks[0].clone();
    for (index, (block_id, tier, ids)) in pushed.iter().enumerate() {
        let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
        let by_raw = &state.message_refs.by_raw;
        let block = Block {
            block_id: (*block_id).to_owned(),
            run_id: format!("r{}", index + 2),
            tier: *tier,
            direct_message_ids: ids.clone(),
            effective_message_ids: ids.clone(),
            compressed_tokens: ids.len() as u64 * 100,
            start_ref: by_raw.get(&ids[0]).cloned(),
            end_ref: by_raw.get(&ids[ids.len() - 1]).cloned(),
            ..base.clone()
        };
        state.blocks.push(block);
    }
    let count = pushed.len() as u32 + 1;
    state.next_block_id = count + 1;
    state.next_run_id = count + 1;
    state.stats.compression_count = u64::from(count);
    state
}

fn covered(state: &State) -> HashSet<String> {
    state
        .covered_message_ids()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn lost(before: &HashSet<String>, state: &State) -> Vec<String> {
    let after = covered(state);
    before
        .iter()
        .filter(|id| !after.contains(*id))
        .cloned()
        .collect()
}

fn merge(messages: &[Message], state: &State, start: &str, end: &str) -> ApplyOutcome {
    apply(
        &[range(start, end, &summary())],
        messages,
        state,
        &merge_config(),
    )
}

fn block<'a>(state: &'a State, id: &str) -> &'a Block {
    state.block(id).expect("block")
}

fn ids(names: &[&str]) -> HashSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn as_set(ids: &[String]) -> HashSet<String> {
    ids.iter().cloned().collect()
}

// --- #4484: a merge preserves existing coverage ---

fn shared_head() -> Vec<Message> {
    vec![
        user("old-user", &long(150)),
        assistant("old-assistant", "Previous work finished."),
        reasoning("reasoning", "Inspect the current state."),
        call(
            "read-call",
            "Read",
            "read-1",
            r#"{"path":"/tmp/example.txt"}"#,
        ),
    ]
}

fn protected_rule_fixture() -> (Vec<Message>, State, Vec<&'static str>) {
    let mut messages = shared_head();
    messages.extend([
        call("rule-call", "acp_rule", "rule-1", r#"{"operation":"list"}"#),
        result("read-result", "Read", "read-1", &long(150)),
        result("rule-result", "acp_rule", "rule-1", "No rules recorded."),
        user("new-user", "Continue."),
    ]);
    let tools = vec!["read-call", "read-result"];
    let state = seeded(
        &messages,
        &["rule-call", "rule-result"],
        &[("b2", 1, &tools)],
    );
    (messages, state, tools)
}

fn parallel_tail_fixture() -> (Vec<Message>, State, Vec<&'static str>) {
    let mut messages = shared_head();
    messages.extend([
        call("bash-call", "Bash", "bash-1", r#"{"command":"true"}"#),
        call("todo-call", "TodoList", "todo-1", r#"{"todos":[]}"#),
        result("read-result", "Read", "read-1", &long(150)),
        result("bash-result", "Bash", "bash-1", &long(150)),
        result("todo-result", "TodoList", "todo-1", "Updated."),
        user("new-user", "Continue."),
    ]);
    let tools = vec!["read-call", "bash-call", "read-result", "bash-result"];
    let state = seeded(&messages, &[], &[("b2", 1, &tools)]);
    (messages, state, tools)
}

#[test]
fn merging_keeps_the_read_pair_folded_around_a_protected_rule() {
    for (messages, state, tools) in [protected_rule_fixture(), parallel_tail_fixture()] {
        let before = covered(&state);
        let outcome = merge(&messages, &state, "b1", "b2");
        assert!(
            outcome.result.errors.is_empty(),
            "{:?}",
            outcome.result.errors
        );
        assert_eq!(outcome.result.blocks_created, 1);
        assert_eq!(lost(&before, &outcome.state), Vec::<String>::new());
        let after = covered(&outcome.state);
        for id in tools {
            assert!(after.contains(id), "{id} must stay covered");
        }
        assert!(compute_integrity_withdrawals(&messages, &after, None)
            .withdrawn
            .is_empty());
    }
}

fn mid_pair_fixture() -> (Vec<Message>, State) {
    let messages = vec![
        user("old-user", &long(150)),
        assistant("old-assistant", "Previous work finished."),
        call(
            "read-call",
            "Read",
            "read-1",
            r#"{"path":"/tmp/example.txt"}"#,
        ),
        result("read-result", "Read", "read-1", &long(150)),
        user("new-user", "Continue."),
    ];
    let state = seeded(&messages, &[], &[("b2", 1, &["read-call"])]);
    (messages, state)
}

#[test]
fn a_merge_completes_around_a_mid_pair_block() {
    let (messages, state) = mid_pair_fixture();
    let before = covered(&state);
    let outcome = merge(&messages, &state, "b1", "b2");
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result);
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(lost(&before, &outcome.state), Vec::<String>::new());
    let b2 = block(&outcome.state, "b2");
    assert!(b2.active, "the defective block stays active");
    assert_eq!(as_set(&b2.effective_message_ids), ids(&["read-call"]));
    assert!(!block(&outcome.state, "b1").active, "b1 is consumed");
    let merged = outcome
        .state
        .blocks
        .iter()
        .find(|block| block.active && block.block_id != "b2")
        .expect("a merged block");
    assert!(!merged
        .effective_message_ids
        .contains(&"read-call".to_owned()));
    assert!(merged
        .effective_message_ids
        .contains(&"old-user".to_owned()));
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.contains("b2")),
        "{:?}",
        outcome.result.warnings
    );
}

#[test]
fn a_defective_block_alone_recovers_by_folding_the_stranded_result() {
    let (messages, state) = mid_pair_fixture();
    let before = covered(&state);
    let outcome = merge(&messages, &state, "b2", "b2");
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result);
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Extended this merge to fold stranded partner(s)")),
        "{:?}",
        outcome.result.warnings
    );
    let merged = outcome
        .state
        .blocks
        .iter()
        .find(|block| {
            block.active
                && block
                    .effective_message_ids
                    .contains(&"read-result".to_owned())
        })
        .expect("the recovered merge");
    assert!(merged
        .effective_message_ids
        .contains(&"read-call".to_owned()));
    assert!(!block(&outcome.state, "b2").active);
    assert_eq!(lost(&before, &outcome.state), Vec::<String>::new());
}

#[test]
fn a_clean_merge_loses_nothing() {
    let messages = [
        user("old-user", &long(150)),
        assistant("old-assistant", "Previous work finished."),
        user("mid-user", &long(150)),
        assistant("mid-assistant", "More prior context."),
        user("new-user", "Continue."),
    ];
    let state = seeded(&messages, &[], &[("b2", 1, &["mid-user", "mid-assistant"])]);
    let before = covered(&state);
    let outcome = merge(&messages, &state, "b1", "b2");
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(lost(&before, &outcome.state), Vec::<String>::new());
}

// --- #5789: the tier-2 merge recovery ladder ---

fn user_shape_fixture() -> (Vec<Message>, State) {
    let messages = vec![
        user("u0", &long(150)),
        assistant("a0", "Previous work finished."),
        reasoning("r1", "Inspect the current state."),
        call("k1", "Read", "c1", r#"{"path":"/tmp/a.txt"}"#),
        result("l1", "Read", "c1", &long(150)),
        user("u1", &long(150)),
        assistant("a1", "More prior context."),
        reasoning("r2", "Check the tail."),
        call("k2", "Read", "c2", r#"{"path":"/tmp/b.txt"}"#),
        result("l2", "Read", "c2", &long(150)),
        assistant("t2", "Tail turn done."),
        user("new-user", "Continue."),
    ];
    let state = seeded(
        &messages,
        &[],
        &[
            ("b2", 1, &["r1", "k1", "l1"]),
            ("b3", 1, &["u1", "a1"]),
            ("b4", 1, &["r2", "k2"]),
        ],
    );
    (messages, state)
}

#[test]
fn a_tail_cut_poison_block_stays_active_while_the_merge_succeeds_around_it() {
    let (messages, state) = user_shape_fixture();
    let before = covered(&state);
    let outcome = merge(&messages, &state, "b1", "b4");
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1, "{:?}", outcome.result);
    assert_eq!(lost(&before, &outcome.state), Vec::<String>::new());
    assert!(
        block(&outcome.state, "b4").active,
        "the poison block stays active"
    );
    for id in ["b1", "b2", "b3"] {
        assert!(!block(&outcome.state, id).active, "{id} is consumed");
    }
    let merged = outcome
        .state
        .blocks
        .iter()
        .find(|block| block.active && block.block_id != "b4")
        .expect("a merged block");
    assert_eq!(merged.tier, 2);
    assert_eq!(
        as_set(&merged.effective_message_ids),
        ids(&["u0", "a0", "r1", "k1", "l1", "u1", "a1"])
    );
    let mut consumed = merged.direct_block_ids.clone();
    consumed.sort();
    assert_eq!(consumed, ["b1", "b2", "b3"]);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.contains("b4")),
        "{:?}",
        outcome.result.warnings
    );
    let pre = compute_integrity_withdrawals(&messages, &before, None).withdrawn;
    let post = compute_integrity_withdrawals(&messages, &covered(&outcome.state), None).withdrawn;
    for id in post {
        assert!(
            pre.contains(&id),
            "the merge introduced a violation for {id}"
        );
    }
}

#[test]
fn a_block_self_range_extends_to_the_live_result_and_closes_the_turn() {
    let (messages, state) = user_shape_fixture();
    let before = covered(&state);
    let outcome = merge(&messages, &state, "b4", "b4");
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result);
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Extended this merge to fold stranded partner(s)")),
        "{:?}",
        outcome.result.warnings
    );
    assert!(outcome
        .state
        .blocks
        .iter()
        .any(|block| block.active && block.effective_message_ids.contains(&"l2".to_owned())));
    assert!(!block(&outcome.state, "b4").active);
    assert_eq!(lost(&before, &outcome.state), Vec::<String>::new());
    let again = merge(&messages, &outcome.state, "b4", "b4");
    assert!(again.result.errors.is_empty(), "{:?}", again.result);
}

#[test]
fn a_conflict_owned_by_a_surviving_higher_tier_block_is_inherited() {
    let messages = [
        user("u0", &long(150)),
        assistant("a0", "Previous work finished."),
        reasoning("r2", "Check the tail."),
        call("k2", "Read", "c2", r#"{"path":"/tmp/b.txt"}"#),
        user("u1", &long(150)),
        assistant("a1", "More prior context."),
        result("l2", "Read", "c2", &long(150)),
        assistant("t2", "Tail turn done."),
        user("new-user", "Continue."),
    ];
    let state = seeded(
        &messages,
        &[],
        &[("b2", 2, &["r2", "k2"]), ("b3", 1, &["u1", "a1"])],
    );
    let before = covered(&state);
    let outcome = merge(&messages, &state, "b1", "b3");
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result);
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(lost(&before, &outcome.state), Vec::<String>::new());
    let b2 = block(&outcome.state, "b2");
    assert!(b2.active, "the surviving poison block is untouched");
    assert_eq!(as_set(&b2.effective_message_ids), ids(&["r2", "k2"]));
    let merged = outcome
        .state
        .blocks
        .iter()
        .find(|block| block.active && block.block_id != "b2")
        .expect("a merged block");
    assert_eq!(merged.tier, 2);
    assert_eq!(
        as_set(&merged.effective_message_ids),
        ids(&["u0", "a0", "u1", "a1"])
    );
    let mut consumed = merged.direct_block_ids.clone();
    consumed.sort();
    assert_eq!(consumed, ["b1", "b3"]);
    assert!(
        outcome.result.warnings.iter().any(|warning| {
            warning.contains("Inherited a pre-existing turn-integrity conflict")
                && warning.contains("m00007")
        }),
        "{:?}",
        outcome.result.warnings
    );
}

// --- #5919: a shredded merge ---

/// `b2` holds the reasoning (tier 2), `b3` the text and the call; the
/// call's result `l` stays visible.
fn shredded(result_blocked: bool, result_tool: &str) -> (Vec<Message>, State) {
    let body = long(120);
    let messages = vec![
        user("u0", &body),
        assistant("a0", "Previous work finished."),
        reasoning("r", &body),
        assistant("t", &body),
        call("k", "Read", "c1", r#"{"path":"/tmp/a.txt"}"#),
        result("l", result_tool, "c1", &body),
        assistant("far", "Far tail."),
        user("newuser", "Continue."),
    ];
    let blocked: &[&str] = if result_blocked { &["l"] } else { &[] };
    let state = seeded(
        &messages,
        blocked,
        &[("b2", 2, &["r"]), ("b3", 1, &["t", "k"])],
    );
    (messages, state)
}

#[test]
fn a_foldable_stranded_partner_outside_the_span_is_folded_in() {
    let (messages, state) = shredded(false, "Read");
    let outcome = merge(&messages, &state, "b2", "b3");
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let merged = block(&outcome.state, "b4");
    for id in ["l", "t", "k"] {
        assert!(
            merged.effective_message_ids.contains(&id.to_owned()),
            "{id}"
        );
    }
    assert!(!block(&outcome.state, "b3").active);
    assert!(block(&outcome.state, "b2").active);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Extended this merge to fold stranded partner(s)")),
        "{:?}",
        outcome.result.warnings
    );
}

#[test]
fn a_blocked_stranded_partner_makes_the_merge_a_no_op() {
    let (messages, state) = shredded(true, "Read");
    let outcome = merge(&messages, &state, "b2", "b3");
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(block(&outcome.state, "b3").active);
    let skip = outcome
        .result
        .warnings
        .iter()
        .find(|warning| warning.starts_with("Merge of b3 skipped"))
        .expect("the skip warning");
    assert!(skip.contains("cannot be closed now"), "{skip}");
    assert!(skip.contains("Do not retry this merge"), "{skip}");
    assert!(skip.contains("BLOCKED"), "{skip}");
    assert!(!outcome
        .result
        .warnings
        .iter()
        .any(|warning| warning.contains("age into coverage")));
}

#[test]
fn a_protected_tool_stranded_partner_makes_the_merge_a_no_op() {
    let (messages, state) = shredded(false, "acp_rule");
    let outcome = merge(&messages, &state, "b2", "b3");
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(block(&outcome.state, "b3").active);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Merge of b3 skipped")
                && warning.contains("Do not retry")),
        "{:?}",
        outcome.result.warnings
    );
}

#[test]
fn a_foldable_partner_inside_a_message_range_folds_cleanly() {
    let (messages, state) = shredded(false, "Read");
    let outcome = merge(&messages, &state, "m00005", "m00006");
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(block(&outcome.state, "b4")
        .effective_message_ids
        .contains(&"l".to_owned()));
    assert!(block(&outcome.state, "b3").active);
}

/// The host's protection flag and the compress exchanges that applied
/// nothing (`message-protection.test.ts`, `issue4289-failed-compress-fold.test.ts`).
mod protected_exchanges {
    use hanafubuki_agent_compression::{
        assign_message_refs, collect_failed_compress_call_ids, is_failed_compress_result,
        is_message_protected, Config, Message, Role, State, BLOCKED_REF,
    };

    use crate::fixtures::{apply, config, range, state_for};
    use crate::protect::{
        call, fold, joined, long_text, protecting, result, sorted, text, VALID_SUMMARY,
    };

    // --- message-protection (the host's flag) ---

    fn catalog(id: &str, role: Role) -> Message {
        Message {
        protected: true,
        ..Message::text(
            id,
            role,
            "<system-reminder> <available_skills> skill-a: does A; skill-b: does B </system-reminder>",
        )
    }
    }

    #[test]
    fn a_host_protected_message_stays_out_of_the_block() {
        for role in [Role::User, Role::Assistant] {
            let messages = [
                text("a", &long_text()),
                catalog("catalog", role),
                text("d", &long_text()),
            ];
            let outcome = fold(&messages, &config(), "m00001", "m00003", VALID_SUMMARY);
            assert_eq!(outcome.result.blocks_created, 1);
            assert!(outcome.result.errors.is_empty());
            let block = &outcome.state.blocks[0];
            assert!(!block.direct_message_ids.contains(&"catalog".to_owned()));
            assert!(!block.effective_message_ids.contains(&"catalog".to_owned()));
            assert!(block.direct_message_ids.contains(&"a".to_owned()));
            assert!(block.direct_message_ids.contains(&"d".to_owned()));
            assert_eq!(block.summary, VALID_SUMMARY);
        }
    }

    #[test]
    fn without_the_flag_the_same_message_folds() {
        let unflagged = Message {
            protected: false,
            ..catalog("catalog", Role::User)
        };
        let messages = [text("a", &long_text()), unflagged, text("d", &long_text())];
        let outcome = fold(&messages, &config(), "m00001", "m00003", VALID_SUMMARY);
        assert!(outcome.state.blocks[0]
            .effective_message_ids
            .contains(&"catalog".to_owned()));
    }

    #[test]
    fn a_host_protected_message_gets_a_blocked_ref() {
        let messages = [
            text("a", &long_text()),
            catalog("catalog", Role::User),
            text("d", &long_text()),
        ];
        let mut state = State::default();
        assign_message_refs(&mut state, &messages, &config());
        let by_raw = &state.message_refs.by_raw;
        assert_eq!(by_raw["catalog"], BLOCKED_REF);
        assert_ne!(by_raw["a"], BLOCKED_REF);
        assert_ne!(by_raw["d"], BLOCKED_REF);
    }

    #[test]
    fn a_numeric_ref_from_before_protection_does_not_make_it_foldable() {
        // The refs are minted while nothing is protected, then the host flags it.
        let messages = [
            text("a", &long_text()),
            catalog("catalog", Role::User),
            text("d", &long_text()),
        ];
        let unflagged: Vec<Message> = messages
            .iter()
            .map(|message| Message {
                protected: false,
                ..message.clone()
            })
            .collect();
        let state = state_for(&unflagged);
        assert_ne!(state.message_refs.by_raw["catalog"], BLOCKED_REF);
        let outcome = apply(
            &[range("m00001", "m00003", VALID_SUMMARY)],
            &messages,
            &state,
            &config(),
        );
        let block = &outcome.state.blocks[0];
        assert!(!block.effective_message_ids.contains(&"catalog".to_owned()));
        assert!(block.effective_message_ids.contains(&"a".to_owned()));
        assert!(block.effective_message_ids.contains(&"d".to_owned()));
    }

    #[test]
    fn a_range_of_protected_messages_only_is_a_clean_no_op() {
        let messages = [catalog("catalog", Role::User)];
        let outcome = fold(&messages, &config(), "m00001", "m00001", VALID_SUMMARY);
        assert_eq!(outcome.result.blocks_created, 0);
        assert!(outcome.result.errors.is_empty());
        assert!(outcome.result.noop);
        assert!(joined(&outcome.result.warnings).contains("protected or filtered"));
    }

    // --- #4289/#4307: compress exchanges that applied nothing fold ---

    fn compress_call(id: &str, call_id: &str, arguments: &str) -> Message {
        call(id, "compress", call_id, arguments)
    }

    fn compress_result(id: &str, call_id: &str, receipt: &str) -> Message {
        result(id, "compress", call_id, receipt)
    }

    fn summary_args(start: &str, end: &str) -> String {
        format!(
            r#"{{"content":[{{"startId":"{start}","endId":"{end}","summary":"{VALID_SUMMARY}"}}]}}"#
        )
    }

    fn failed_session() -> Vec<Message> {
        let long = long_text();
        vec![
        text("u1", &long),
        compress_call("fc1", "call-failed", &summary_args("m00001", "m00002")),
        compress_result("fr1", "call-failed", "[Compression FAILED: Requested range(s) cannot be anchored. Nothing compressed. [cause: gate:none-resolved]]"),
        text("u2", &long),
        compress_call("sc1", "call-ok", &summary_args("m00001", "m00001")),
        compress_result("sr1", "call-ok", "[Compressed m00001–m00001 → 1 block(s), ~1234 tokens saved.]"),
        text("u3", &long),
    ]
    }

    fn refused_session() -> Vec<Message> {
        let long = long_text();
        vec![
        text("u1", &long),
        compress_call("rc1", "call-refused-1", &summary_args("m00001", "m00002")),
        compress_result("rr1", "call-refused-1", "[Compression refused: no net shrink — the summary you supplied is not smaller than the range it replaces (committing it leaves the context size unchanged). Nothing was folded.]"),
        text("u2", &long),
        compress_call("rc2", "call-refused-2", &summary_args("m00001", "m00002")),
        compress_result("rr2", "call-refused-2", "[Compression refused: coverage regression — folding these ranges would leave 1 already-folded message(s) without any covering block.]"),
        text("u3", &long),
        compress_call("sc1", "call-ok", &summary_args("m00001", "m00001")),
        compress_result("sr1", "call-ok", "[Compressed m00001–m00001 → 1 block(s), ~1234 tokens saved.]"),
        text("u4", &long),
    ]
    }

    fn exchange_config() -> Config {
        protecting(&["nomatch-*"])
    }

    fn minted(messages: &[Message], config: &Config) -> State {
        let mut state = State::default();
        assign_message_refs(&mut state, messages, config);
        state
    }

    #[test]
    fn failed_and_refused_exchanges_get_numeric_refs() {
        let state = minted(&failed_session(), &exchange_config());
        let by_raw = &state.message_refs.by_raw;
        assert_ne!(by_raw["fc1"], BLOCKED_REF);
        assert_ne!(by_raw["fr1"], BLOCKED_REF);
        assert_eq!(by_raw["sc1"], BLOCKED_REF);
        assert_eq!(by_raw["sr1"], BLOCKED_REF);

        let state = minted(&refused_session(), &exchange_config());
        let by_raw = &state.message_refs.by_raw;
        for id in ["rc1", "rr1", "rc2", "rr2"] {
            assert_ne!(by_raw[id], BLOCKED_REF, "{id}");
        }
        assert_eq!(by_raw["sc1"], BLOCKED_REF);
        assert_eq!(by_raw["sr1"], BLOCKED_REF);

        // The path-pattern (pairing) layout too.
        let state = minted(&failed_session(), &protecting(&["skill/*"]));
        let by_raw = &state.message_refs.by_raw;
        assert_ne!(by_raw["fc1"], BLOCKED_REF);
        assert_ne!(by_raw["fr1"], BLOCKED_REF);
        assert_eq!(by_raw["sc1"], BLOCKED_REF);
        assert_eq!(by_raw["sr1"], BLOCKED_REF);
    }

    #[test]
    fn failed_and_refused_exchanges_fold_while_the_successful_one_stays() {
        for (messages, end, folded) in [
            (failed_session(), "u3", vec!["fc1", "fr1"]),
            (refused_session(), "u4", vec!["rc1", "rr1", "rc2", "rr2"]),
        ] {
            let config = exchange_config();
            let state = minted(&messages, &config);
            let by_raw = &state.message_refs.by_raw;
            let outcome = apply(
                &[range(&by_raw["u1"], &by_raw[end], VALID_SUMMARY)],
                &messages,
                &state,
                &config,
            );
            assert_eq!(outcome.result.blocks_created, 1, "{:?}", outcome.result);
            let block = &outcome.state.blocks[0];
            for id in folded {
                assert!(
                    block.direct_message_ids.contains(&id.to_owned()),
                    "{id} folds"
                );
            }
            assert!(!block.effective_message_ids.contains(&"sc1".to_owned()));
            assert!(!block.effective_message_ids.contains(&"sr1".to_owned()));
        }
    }

    #[test]
    fn a_pending_or_partial_compress_call_stays_protected() {
        let pending = [
            text("u1", &long_text()),
            compress_call("pc1", "call-pending", "{}"),
            text("u2", &long_text()),
        ];
        assert_eq!(
            minted(&pending, &exchange_config()).message_refs.by_raw["pc1"],
            BLOCKED_REF
        );

        let partial = [
        text("u1", &long_text()),
        compress_call("pc1", "call-partial", "{}"),
        compress_result(
            "pr1",
            "call-partial",
            "[Compressed m00001–m00002 → 1 block(s), ~900 tokens saved.\npartial: 1 applied 1 rejected (kind=ok) reasons=[entry 2: line entry: missing summary after the refs header line]]",
        ),
        text("u2", &long_text()),
    ];
        let by_raw = minted(&partial, &exchange_config()).message_refs.by_raw;
        assert_eq!(by_raw["pc1"], BLOCKED_REF);
        assert_eq!(by_raw["pr1"], BLOCKED_REF);
    }

    #[test]
    fn legacy_blocked_refs_of_a_failed_exchange_heal_to_numbers() {
        let messages = failed_session();
        let config = exchange_config();
        let mut state = State::default();
        assign_message_refs(&mut state, &messages[..1], &config);
        // Before the fix every compress exchange was protected.
        let legacy: Vec<Message> = messages
            .iter()
            .map(|message| Message {
                protected: message.tool_name.as_deref() == Some("compress"),
                ..message.clone()
            })
            .collect();
        assign_message_refs(&mut state, &legacy, &config);
        assert_eq!(state.message_refs.by_raw["fc1"], BLOCKED_REF);

        assign_message_refs(&mut state, &messages, &config);
        assert_ne!(state.message_refs.by_raw["fc1"], BLOCKED_REF);
        assert_ne!(state.message_refs.by_raw["fr1"], BLOCKED_REF);
        assert_eq!(state.message_refs.by_raw["sc1"], BLOCKED_REF);
    }

    #[test]
    fn failed_receipts_are_recognized_by_every_zero_applied_prefix() {
        let failed = compress_result("r", "c1", "[Compression FAILED: nope]");
        assert!(is_failed_compress_result(&failed));
        assert!(!is_failed_compress_result(&Message {
            tool_name: Some("acp_rule".to_owned()),
            ..failed.clone()
        }));
        assert!(!is_failed_compress_result(&Message {
            text: "[Compressed m00001–m00002 → 1 block(s), ~10 tokens saved.]".to_owned(),
            ..failed.clone()
        }));
        assert!(!is_failed_compress_result(&Message {
            text: String::new(),
            ..failed
        }));

        let no_net_shrink = compress_result("r", "c1", "[Compression refused: no net shrink — the summary you supplied is not smaller than the range it replaces (committing it leaves the context size unchanged). Nothing was folded.]");
        let coverage = compress_result("r", "c2", "[Compression refused: coverage regression — folding these ranges would leave 1 already-folded message(s) without any covering block.]");
        assert!(is_failed_compress_result(&no_net_shrink));
        assert!(is_failed_compress_result(&coverage));

        let messages = [
            compress_call("c1", "call-refused", "{}"),
            compress_result("r1", "call-refused", &no_net_shrink.text),
            compress_call("c2", "call-failed", "{}"),
            compress_result("r2", "call-failed", "[Compression FAILED: nope]"),
            compress_call("c3", "call-ok", "{}"),
            compress_result(
                "r3",
                "call-ok",
                "[Compressed m00001–m00001 → 1 block(s), ~10 tokens saved.]",
            ),
        ];
        assert_eq!(
            sorted(&collect_failed_compress_call_ids(&messages)),
            ["call-failed", "call-refused"]
        );
        let failed_ids = collect_failed_compress_call_ids(&messages);
        assert!(!is_message_protected(
            &messages[0],
            &exchange_config(),
            &failed_ids
        ));
        assert!(is_message_protected(
            &messages[4],
            &exchange_config(),
            &failed_ids
        ));
    }
}
