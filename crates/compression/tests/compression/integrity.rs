// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `issue684-turn-integrity.test.ts`, `issue5751-tool-pair-widening.test.ts`,
// `issue5531-stranded-reasoning-residue.test.ts`,
// `issue3046-duplicate-tool-call-ids.test.ts` and `issue3953-straddle-truncate.test.ts`.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    adjust_boundaries_for_reasoning_pairs, adjust_boundaries_for_tool_pairs, apply_compression,
    compute_integrity_withdrawals, compute_turn_groups, project_unit, prune, ApplyInput,
    ApplyOutcome, Block, Config, FatalKind, Kind, Message, ProjectionContext, Role, State,
    UnitRequest,
};

use crate::fixtures::{apply, range, state_for};

fn user(id: &str) -> Message {
    Message::text(id, Role::User, format!("u-{id} {}", "x".repeat(200)))
}

fn reasoning(id: &str) -> Message {
    Message::reasoning(id, format!("think-{id} {}", "t".repeat(400)))
}

fn assistant(id: &str) -> Message {
    Message::text(
        id,
        Role::Assistant,
        format!("text-{id} {}", "x".repeat(200)),
    )
}

fn tool_call(id: &str, call_id: &str, tool: &str) -> Message {
    Message::tool_call(id, call_id, tool, format!("call-{call_id}"))
}

fn tool_result(id: &str, call_id: &str, tool: &str) -> Message {
    Message {
        role: Role::User,
        ..Message::tool_result(
            id,
            call_id,
            tool,
            format!("result-{call_id} {}", "r".repeat(200)),
        )
    }
}

fn short_text(id: &str) -> Message {
    Message::text(id, Role::User, format!("text-{id}"))
}

fn short_call(id: &str, call_id: &str) -> Message {
    tool_call(id, call_id, "read")
}

fn short_result(id: &str, call_id: &str) -> Message {
    Message {
        role: Role::User,
        ..Message::tool_result(id, call_id, "read", format!("result-{call_id}"))
    }
}

/// The kernel's `defaultConfig(150000)` with the minimum range and summary
/// at 1.
fn integrity_config() -> Config {
    let mut config = Config::new(150_000);
    config.compress.min_compress_range = 1;
    config.compress.min_summary_length = 1;
    config
}

fn open_config() -> Config {
    let mut config = Config::new(100_000);
    config.compress.min_compress_range = 0;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

fn active_block(state: &State) -> &Block {
    state
        .blocks
        .iter()
        .find(|block| block.active)
        .expect("an active block")
}

fn covers(block: &Block, id: &str) -> bool {
    block
        .effective_message_ids
        .iter()
        .any(|covered| covered == id)
}

fn view_ids(messages: &[Message], state: &State) -> HashSet<String> {
    prune(messages, state)
        .into_iter()
        .map(|message| message.id)
        .collect()
}

fn ids_of(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn refs_of(state: &State, ids: &[&str]) -> HashSet<String> {
    ids.iter()
        .map(|id| state.message_refs.by_raw[*id].clone())
        .collect()
}

fn apply_in_zone(
    messages: &[Message],
    state: &State,
    config: &Config,
    zone: &HashSet<String>,
    start: &str,
    end: &str,
    summary: &str,
) -> ApplyOutcome {
    let ranges = [range(start, end, summary)];
    apply_compression(&ApplyInput {
        ranges: &ranges,
        messages,
        state,
        config,
        protected_refs: Some(zone),
        now_ms: 0,
    })
}

// --- #684: turn integrity ---

fn burst_turn() -> Vec<Message> {
    vec![
        user("m00001"),
        user("m00002"),
        reasoning("m00003"),
        tool_call("m00004", "call_00_x", "pwsh"),
        tool_call("m00005", "call_01_x", "pwsh"),
        tool_result("m00006", "call_00_x", "pwsh"),
        tool_result("m00007", "call_01_x", "pwsh"),
        user("m00008"),
        user("m00009"),
    ]
}

#[test]
fn a_reasoning_run_its_burst_and_its_results_are_one_turn() {
    let groups = compute_turn_groups(&burst_turn());
    let turn = groups
        .iter()
        .find(|group| group.contains(&"m00003".to_owned()))
        .expect("the turn");
    let members: HashSet<String> = turn.iter().cloned().collect();
    assert_eq!(
        members,
        ids_of(&["m00003", "m00004", "m00005", "m00006", "m00007"])
    );
}

#[test]
fn a_burst_without_reasoning_groups_with_its_results() {
    let messages = [
        user("m1"),
        tool_call("m2", "c1", "pwsh"),
        tool_result("m3", "c1", "pwsh"),
        user("m4"),
    ];
    assert_eq!(compute_turn_groups(&messages), [["m2", "m3"]]);
}

#[test]
fn orphan_reasoning_belongs_to_no_turn() {
    let messages = [user("m1"), reasoning("m2"), user("m3")];
    assert!(compute_turn_groups(&messages).is_empty());
}

#[test]
fn the_forward_pull_covers_the_whole_burst() {
    let (_, end) = adjust_boundaries_for_reasoning_pairs(0, 2, &burst_turn());
    assert_eq!(end, 4);
}

#[test]
fn a_carve_that_keeps_one_call_withdraws_the_whole_turn() {
    let messages = burst_turn();
    let state = state_for(&messages);
    let zone = refs_of(&state, &["m00005", "m00007"]);
    let outcome = apply_in_zone(
        &messages,
        &state,
        &integrity_config(),
        &zone,
        "m00001",
        "m00007",
        &"s".repeat(400),
    );
    let block = active_block(&outcome.state);
    for id in ["m00003", "m00004", "m00005", "m00006", "m00007"] {
        assert!(!covers(block, id), "{id} must not fold");
    }
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.contains("turn")),
        "{:?}",
        outcome.result.warnings
    );
    let visible = view_ids(&messages, &outcome.state);
    for id in ["m00003", "m00004", "m00005", "m00006", "m00007"] {
        assert!(visible.contains(id), "{id} must stay visible");
    }
}

#[test]
fn a_range_that_is_exactly_the_split_turn_fails_clearly() {
    let messages = burst_turn();
    let state = state_for(&messages);
    let zone = refs_of(&state, &["m00005", "m00007"]);
    let outcome = apply_in_zone(
        &messages,
        &state,
        &integrity_config(),
        &zone,
        "m00003",
        "m00007",
        &"s".repeat(400),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(
        outcome.result.errors[0].contains("split 1 turn"),
        "{:?}",
        outcome.result.errors
    );
}

#[test]
fn folding_call_and_result_while_keeping_reasoning_and_text_stays_allowed() {
    let messages = [
        user("m1"),
        reasoning("m2"),
        Message::text("m3", Role::Assistant, format!("TEXT-A {}", "x".repeat(200))),
        tool_call("m4", "ca", "pwsh"),
        tool_result("m5", "ca", "pwsh"),
        user("m6"),
        user("m7"),
    ];
    let state = state_for(&messages);
    let outcome = apply_in_zone(
        &messages,
        &state,
        &integrity_config(),
        &HashSet::new(),
        "m00004",
        "m00005",
        &"s".repeat(400),
    );
    assert_eq!(
        outcome.result.blocks_created, 1,
        "errors={:?}",
        outcome.result.errors
    );
    let block = active_block(&outcome.state);
    assert!(covers(block, "m4") && covers(block, "m5"));
    assert!(
        !outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.contains("turn")),
        "{:?}",
        outcome.result.warnings
    );
}

// --- #5751: id-exact tool-pair widening ---

#[test]
fn interleaved_pairs_chain_to_a_fixed_point() {
    let mut messages = vec![short_call("a", "c1"), short_call("b", "c2")];
    messages.extend((0..28).map(|index| short_text(&format!("gap1-{index}"))));
    messages.push(short_result("r1", "c1"));
    messages.extend((0..29).map(|index| short_text(&format!("gap2-{index}"))));
    messages.push(short_result("r2", "c2"));
    assert_eq!(
        adjust_boundaries_for_tool_pairs(0, 0, &messages),
        (0, messages.len() - 1)
    );
}

#[test]
fn an_interleaved_mate_is_not_stranded_by_a_gap() {
    let messages = [
        short_call("a", "c1"),
        short_call("b", "c2"),
        short_result("r1", "c1"),
        short_text("gap"),
        short_result("r2", "c2"),
    ];
    assert_eq!(adjust_boundaries_for_tool_pairs(0, 1, &messages), (0, 4));
}

#[test]
fn a_straddling_range_folds_a_distant_pair_whole() {
    let mut messages = vec![short_text("m0"), short_call("m1", "c1")];
    messages.extend((2..30).map(|index| short_text(&format!("m{index}"))));
    messages.push(short_result("m30", "c1"));
    messages.push(short_text("m31"));
    let state = state_for(&messages);
    let outcome = apply(
        &[range(
            "m00001",
            "m00002",
            "Fold the first exchange including its late-arriving result.",
        )],
        &messages,
        &state,
        &open_config(),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = active_block(&outcome.state);
    assert!(covers(block, "m1"), "tool-call folds");
    assert!(
        covers(block, "m30"),
        "the distant result folds with its call"
    );
    let visible = view_ids(&messages, &outcome.state);
    assert!(!visible.contains("m1") && !visible.contains("m30"));
}

#[test]
fn every_range_of_a_tool_dense_region_lands() {
    let mut messages = Vec::new();
    for pair in 0..8 {
        messages.push(short_call(&format!("call{pair}"), &format!("cc{pair}")));
        messages.extend((0..24).map(|index| short_text(&format!("f{pair}-{index}"))));
        messages.push(short_result(&format!("res{pair}"), &format!("cc{pair}")));
    }
    let state = state_for(&messages);
    let outcome = apply(
        &[range(
            "m00001",
            "m00025",
            "First exchanges of the tool-dense region: the opening reads, their late results, and filler turns.",
        )],
        &messages,
        &state,
        &open_config(),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    let block = active_block(&outcome.state);
    assert!(covers(block, "call0") && covers(block, "res0"));
    let visible = view_ids(&messages, &outcome.state);
    assert!(!visible.contains("call0") && !visible.contains("res0"));
    assert!(visible.contains("call1") && visible.contains("res1"));
}

#[test]
fn block_ranges_still_skip_pair_widening() {
    let messages = [
        short_text("m0"),
        short_call("m1", "c1"),
        short_result("m2", "c1"),
        short_text("m3"),
    ];
    let config = open_config();
    let state = state_for(&messages);
    let first = apply(
        &[range(
            "m00001",
            "m00003",
            "First block covering a complete exchange: the call, its result, and filler.",
        )],
        &messages,
        &state,
        &config,
    );
    assert_eq!(first.result.blocks_created, 1);
    let second = apply(
        &[range(
            "b1",
            "b1",
            "Distill the first block into its tier-2 essence, keeping the exchange facts.",
        )],
        &messages,
        &first.state,
        &config,
    );
    assert_eq!(second.result.blocks_created, 1);
    assert_eq!(
        second.state.block("b2").expect("b2").direct_block_ids,
        ["b1"]
    );
}

// --- #5531: a covered call cannot strand its turn's reasoning ---

fn reverse_split_turn() -> Vec<Message> {
    vec![
        user("u1"),
        reasoning("r"),
        assistant("t"),
        tool_call("c", "ca", "pwsh"),
        tool_result("res", "ca", "pwsh"),
        user("u6"),
        user("u7"),
    ]
}

fn stranded_config() -> Config {
    let mut config = integrity_config();
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

/// Folds the call and its result alone, leaving reasoning and text visible.
fn seed_reverse_split(messages: &[Message]) -> State {
    let state = state_for(messages);
    let seed = apply(
        &[range("m00004", "m00005", &"s".repeat(400))],
        messages,
        &state,
        &stranded_config(),
    );
    let block = active_block(&seed.state);
    assert!(covers(block, "c") && covers(block, "res"));
    assert!(!covers(block, "r") && !covers(block, "t"));
    seed.state
}

fn coverage(state: &State) -> HashSet<String> {
    state
        .covered_message_ids()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn without_coverage_as_invisible_the_gate_strands_the_pair() {
    let messages = reverse_split_turn();
    seed_reverse_split(&messages);
    let withdrawn = compute_integrity_withdrawals(&messages, &ids_of(&["r", "t"]), None).withdrawn;
    assert!(withdrawn.contains(&"r".to_owned()) && withdrawn.contains(&"t".to_owned()));
}

#[test]
fn with_coverage_as_invisible_the_stranded_pair_stays_foldable() {
    let messages = reverse_split_turn();
    let state = seed_reverse_split(&messages);
    let covered = coverage(&state);
    let withdrawn =
        compute_integrity_withdrawals(&messages, &ids_of(&["r", "t"]), Some(&covered)).withdrawn;
    assert!(
        !withdrawn.contains(&"r".to_owned()) && !withdrawn.contains(&"t".to_owned()),
        "{withdrawn:?}"
    );
}

#[test]
fn the_projection_judges_the_stranded_pair_foldable() {
    let messages = reverse_split_turn();
    let state = seed_reverse_split(&messages);
    let config = stranded_config();
    let covered = coverage(&state);
    let context = ProjectionContext::new(&messages, &state, &config);
    let by_raw = &state.message_refs.by_raw;
    let request = UnitRequest {
        start_ref: &by_raw["r"],
        end_ref: &by_raw["t"],
        pre_existing_coverage: &covered,
        protected_refs: None,
    };
    let projection = project_unit(&context, &request, &mut Vec::new()).expect("resolves");
    assert_eq!(projection.fatal, None);
    assert!(projection.filtered_ids.contains(&"r".to_owned()));
    assert!(projection.filtered_ids.contains(&"t".to_owned()));
}

#[test]
fn the_stranded_range_folds_without_a_split_error() {
    let messages = reverse_split_turn();
    let state = seed_reverse_split(&messages);
    let outcome = apply(
        &[range(
            "m00002",
            "m00003",
            "reclaiming the stranded turn head",
        )],
        &messages,
        &state,
        &stranded_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.blocks_created >= 1);
    let block = outcome.state.block("b2").expect("a recovery block");
    assert!(covers(block, "r") && covers(block, "t"));
}

#[test]
fn a_visible_call_still_guards_its_reasoning_in_the_projection() {
    let messages = reverse_split_turn();
    let state = state_for(&messages);
    let config = stranded_config();
    let zone = refs_of(&state, &["c", "res"]);
    let context = ProjectionContext::new(&messages, &state, &config);
    let by_raw = &state.message_refs.by_raw;
    let request = UnitRequest {
        start_ref: &by_raw["r"],
        end_ref: &by_raw["t"],
        pre_existing_coverage: &HashSet::new(),
        protected_refs: Some(&zone),
    };
    let projection = project_unit(&context, &request, &mut Vec::new()).expect("resolves");
    let fatal = projection
        .fatal
        .expect("a visible call must keep its reasoning");
    assert_eq!(fatal.kind, FatalKind::Split);
    assert!(fatal.message.contains("split 1 turn"), "{}", fatal.message);
}

#[test]
fn a_visible_call_still_refuses_the_split_when_applying() {
    let messages = reverse_split_turn();
    let state = state_for(&messages);
    let zone = refs_of(&state, &["c", "res"]);
    let outcome = apply_in_zone(
        &messages,
        &state,
        &stranded_config(),
        &zone,
        "m00002",
        "m00003",
        &"s".repeat(400),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(outcome.result.errors[0].contains("split 1 turn"));
}

// --- #3046: duplicate tool-call ids pair by order ---

fn dup_text(id: &str) -> Message {
    Message::text(id, Role::User, id)
}

fn dup_call(id: &str, call_id: &str) -> Message {
    Message::tool_call(id, call_id, "send_message", "{}")
}

fn dup_result(id: &str, call_id: &str) -> Message {
    Message::tool_result(id, call_id, "send_message", "ok")
}

fn folding(ids: &[&str]) -> State {
    let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    State {
        blocks: vec![Block {
            block_id: "b0".to_owned(),
            run_id: "r0".to_owned(),
            summary: "Folded.".to_owned(),
            direct_message_ids: ids.clone(),
            effective_message_ids: ids,
            ..Block::default()
        }],
        ..State::default()
    }
}

/// Every result follows a call of its id, and every call but `compress`
/// is answered later.
fn assert_wire_valid(view: &[Message]) {
    for (index, message) in view.iter().enumerate() {
        let Some(call_id) = &message.tool_call_id else {
            continue;
        };
        let same_id = |other: &Message, kind| {
            other.kind == kind && other.tool_call_id.as_ref() == Some(call_id)
        };
        if message.kind == Kind::ToolResult {
            assert!(
                view[..index]
                    .iter()
                    .any(|prior| same_id(prior, Kind::ToolCall)),
                "result {} precedes any call declaring it",
                message.id
            );
        }
        if message.kind == Kind::ToolCall && message.tool_name.as_deref() != Some("compress") {
            assert!(
                view[index + 1..]
                    .iter()
                    .any(|later| same_id(later, Kind::ToolResult)),
                "call {} has no following response",
                message.id
            );
        }
    }
}

fn view(messages: &[Message], state: &State) -> (Vec<Message>, HashSet<String>) {
    let view = prune(messages, state);
    let ids = view.iter().map(|message| message.id.clone()).collect();
    (view, ids)
}

#[test]
fn folding_the_call_strips_the_stray_result_of_a_reused_id() {
    let messages = [
        dup_text("u0"),
        dup_call("a1", "send_message:0"),
        dup_result("a1r", "send_message:0"),
        dup_text("u1"),
        dup_call("a2", "send_message:0"),
        dup_result("a2r", "send_message:0"),
    ];
    let (out, ids) = view(&messages, &folding(&["a1"]));
    assert!(!ids.contains("a1") && !ids.contains("a1r"));
    assert!(ids.contains("a2") && ids.contains("a2r"));
    assert_wire_valid(&out);
}

#[test]
fn folding_the_result_strips_the_unanswered_call_of_a_reused_id() {
    let messages = [
        dup_text("u0"),
        dup_call("a1", "send_message:1"),
        dup_result("a1r", "send_message:1"),
        dup_text("u1"),
        dup_call("a2", "send_message:1"),
        dup_result("a2r", "send_message:1"),
    ];
    let (out, ids) = view(&messages, &folding(&["a2r"]));
    assert!(!ids.contains("a2r") && !ids.contains("a2"));
    assert!(ids.contains("a1") && ids.contains("a1r"));
    assert_wire_valid(&out);
}

#[test]
fn one_call_answers_several_results_of_its_id() {
    let messages = [
        dup_text("u0"),
        Message::tool_call("c1", "list_agents:0", "send_message", "{}"),
        dup_result("o1", "list_agents:0"),
        dup_result("o2", "list_agents:0"),
        dup_text("f9"),
        dup_text("u5"),
    ];
    let (out, ids) = view(&messages, &folding(&["f9"]));
    assert!(ids.contains("c1") && ids.contains("o1") && ids.contains("o2"));
    assert_wire_valid(&out);
}

#[test]
fn a_valid_duplicate_burst_survives_an_unrelated_fold() {
    let messages = [
        dup_text("u0"),
        dup_call("a1", "X"),
        dup_result("a1r", "X"),
        dup_text("f1"),
        dup_call("a2", "X"),
        dup_result("a2r", "X"),
        dup_text("uEnd"),
    ];
    let (out, ids) = view(&messages, &folding(&["f1"]));
    for keep in ["a1", "a1r", "a2", "a2r"] {
        assert!(ids.contains(keep), "{keep} must survive");
    }
    assert_wire_valid(&out);
}

// --- #3953: an interior protected message truncates the fold ---

fn alternating(count: usize, role: impl Fn(usize) -> Role) -> Vec<Message> {
    (1..=count)
        .map(|index| {
            Message::text(
                format!("p{index}"),
                role(index),
                format!("body {index} ").repeat(120),
            )
        })
        .collect()
}

fn alternating_role(index: usize) -> Role {
    if index % 2 == 1 {
        Role::User
    } else {
        Role::Assistant
    }
}

fn straddle_config(preserve_recent_messages: usize) -> Config {
    let mut config = Config::new(200_000);
    config.preserve_recent_messages = preserve_recent_messages;
    config.preserve_recent_tokens = 0;
    config
}

const STRADDLE_SUMMARY: &str =
    "A long enough summary body to satisfy the minimum summary length gate of fifty characters.";

#[test]
fn an_interior_protected_message_truncates_the_fold() {
    let messages = alternating(10, |index| {
        if index > 7 {
            Role::Assistant
        } else {
            alternating_role(index)
        }
    });
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00002", "m00009", STRADDLE_SUMMARY)],
        &messages,
        &state,
        &straddle_config(1),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(
        active_block(&outcome.state).effective_message_ids,
        ["p2", "p3", "p4", "p5", "p6"]
    );
    let warning = outcome
        .result
        .warnings
        .iter()
        .find(|warning| warning.starts_with("Excluded"))
        .expect("an exclusion warning");
    assert!(warning.to_lowercase().contains("truncated"), "{warning}");
    assert!(warning.contains("stay visible"), "{warning}");
}

#[test]
fn a_tail_brush_keeps_the_advisory_carve_without_truncation() {
    let messages = alternating(8, alternating_role);
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00002", "m00008", STRADDLE_SUMMARY)],
        &messages,
        &state,
        &straddle_config(2),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(
        active_block(&outcome.state).effective_message_ids,
        ["p2", "p3", "p4", "p5", "p6"]
    );
    let warning = outcome
        .result
        .warnings
        .iter()
        .find(|warning| warning.starts_with("Excluded"))
        .expect("an exclusion warning");
    assert!(
        warning.contains("Excluded 2 protected message(s) m00007, m00008"),
        "{warning}"
    );
    assert!(warning.contains("do not target them in another compress call"));
    assert!(!warning.to_lowercase().contains("truncated"), "{warning}");
}

/// The recent zone over tool pairs and its user anchor
/// (`issue2886-recent-pair-integrity.test.ts`, `issue5109-anchor-skip-injections.test.ts`).
mod recent_zone {
    use std::collections::HashSet;

    use hanafubuki_agent_compression::{compute_protected_refs, last_user_anchor, Config, Message};

    use crate::fixtures::state_for;
    use crate::protect::{
        assistant, call, fold, last_block_covers, recent, result, text, unnamed_result,
    };

    // --- #2886: the zone is closed over tool pairs ---

    fn bare_call(id: &str, tool: &str, call_id: &str) -> Message {
        call(id, tool, call_id, "")
    }

    #[test]
    fn a_result_is_protected_with_its_call_in_the_zone() {
        let messages = [
            text("a", "old alpha context"),
            bare_call("b", "pwsh", "c1"),
            result("c", "read", "c1", &"x".repeat(200)),
            text("d", "latest user intent gamma"),
        ];
        let zone = compute_protected_refs(&messages, &state_for(&messages), &recent(2));
        assert!(zone.contains("m00002"));
        assert!(
            zone.contains("m00003"),
            "its paired read result is co-protected"
        );
        assert!(zone.contains("m00004"));
        assert!(!zone.contains("m00001"));
    }

    #[test]
    fn a_call_is_protected_with_its_result_in_the_zone() {
        let messages = [
            text("a", "old alpha context"),
            text("x", "older assistant beta"),
            bare_call("b", "custom_tool", "c1"),
            result("c", "custom_tool", "c1", &"y".repeat(200)),
        ];
        let zone = compute_protected_refs(&messages, &state_for(&messages), &recent(1));
        assert!(zone.contains("m00004"));
        assert!(zone.contains("m00003"));
    }

    #[test]
    fn pairing_binds_by_call_id_without_a_result_name() {
        let messages = [
            text("a", "old alpha context"),
            bare_call("b", "grep", "c1"),
            unnamed_result("c", "c1", &"z".repeat(200)),
        ];
        let zone = compute_protected_refs(&messages, &state_for(&messages), &recent(1));
        assert!(zone.contains("m00003"));
        assert!(zone.contains("m00002"));
    }

    #[test]
    fn an_older_pair_outside_the_zone_is_not_over_protected() {
        let messages = [
            bare_call("b", "read", "c1"),
            result(
                "c",
                "read",
                "c1",
                &format!("spent body {}", "x".repeat(200)),
            ),
            text("d", "newer user delta"),
            text("e", "even newer assistant epsilon"),
            text("f", "latest user zeta"),
        ];
        let zone = compute_protected_refs(&messages, &state_for(&messages), &recent(2));
        assert!(!zone.contains("m00001"));
        assert!(!zone.contains("m00002"));
    }

    #[test]
    fn folding_does_not_strand_a_call_at_the_recent_boundary() {
        let messages = [
            text("bulk1", "first detailed work item alpha content"),
            text("bulk2", "second detailed work item beta content"),
            text("bulk3", "third detailed work item gamma content"),
            text("bulk4", "fourth detailed work item delta content"),
            bare_call("call", "pwsh", "c1"),
            result(
                "result",
                "read",
                "c1",
                &format!("file body {}", "x".repeat(300)),
            ),
            text("last", "latest user intent theta"),
        ];
        let outcome = fold(
            &messages,
            &recent(2),
            "m00001",
            "m00006",
            "fold the spent head",
        );
        assert!(
            outcome.result.errors.is_empty(),
            "{:?}",
            outcome.result.errors
        );
        assert!(outcome.result.blocks_created >= 1);
        assert!(
            !last_block_covers(&outcome, "call"),
            "the recent call stays visible"
        );
        assert!(
            !last_block_covers(&outcome, "result"),
            "its result is co-protected"
        );
    }

    // --- #5109: the recent zone's user anchor ---

    #[test]
    fn the_user_anchor_skips_covered_messages_and_summaries() {
        let messages = [
            text("p1", "earliest user"),
            text("p3", "already folded"),
            assistant("p4", "assistant reply"),
            text("p5", "real human request"),
            assistant("p6", "assistant ack"),
            text(
                "p7",
                "[Compressed conversation section] — stale topic\nbody",
            ),
        ];
        let covered: HashSet<&str> = ["p3"].into_iter().collect();
        assert_eq!(
            last_user_anchor(&messages, &covered).map(|anchor| anchor.id.as_str()),
            Some("p5")
        );
    }

    #[test]
    fn the_literal_last_user_message_anchors_the_zone() {
        let messages = [
            text("p1", "question"),
            assistant("p2", "answer"),
            text("p3", "plain follow-up"),
        ];
        let mut config = Config::new(200_000);
        config.preserve_recent_messages = 1;
        config.preserve_recent_tokens = 0;
        let zone = compute_protected_refs(&messages, &state_for(&messages), &config);
        assert!(zone.contains("m00003"));
    }

    #[test]
    fn the_fold_receipt_names_the_user_anchor() {
        let messages = [
            text("p1", &"start user body ".repeat(120)),
            assistant("p2", &"body two ".repeat(120)),
            text("p3", &"body three ".repeat(120)),
            assistant("p4", &"body four ".repeat(120)),
            text("p5", &"body five ".repeat(120)),
            assistant("p6", &"body six ".repeat(120)),
            text("p7", "current live request: run the migration now"),
            assistant("p8", "assistant starts working"),
        ];
        let mut config = Config::new(200_000);
        config.preserve_recent_messages = 1;
        config.preserve_recent_tokens = 0;
        let outcome = fold(
        &messages,
        &config,
        "m00002",
        "m00007",
        "A long enough summary body to satisfy the minimum summary length gate of fifty characters.",
    );
        assert_eq!(outcome.result.blocks_created, 1, "{:?}", outcome.result);
        assert!(
            !last_block_covers(&outcome, "p7"),
            "the real request never folds"
        );
        let warning = outcome
            .result
            .warnings
            .iter()
            .find(|warning| warning.starts_with("Excluded"))
            .expect("an exclusion warning");
        assert!(
            warning
                .contains(r#"Last-user anchor: m00007 "current live request: run the migration ""#),
            "{warning}"
        );
    }
}
