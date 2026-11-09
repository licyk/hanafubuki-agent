// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `protect-recent.test.ts`, `protected-content.test.ts`,
// `protected-latest.test.ts` and `protected-paths.test.ts`; its helpers serve
// the protection tests in `integrity.rs` and `merge.rs` too.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    assign_message_refs, collect_latest_protected, compute_protected_refs,
    is_message_latest_protected, is_message_protected, is_never_preserve_recent, match_tool_path,
    match_tool_pattern, ApplyOutcome, Config, Message, Role, State, BLOCKED_REF,
};

use crate::fixtures::{apply, config, range, state_for, with_topic};

pub(crate) fn text(id: &str, body: &str) -> Message {
    Message::text(id, Role::User, body)
}

pub(crate) fn assistant(id: &str, body: &str) -> Message {
    Message::text(id, Role::Assistant, body)
}

pub(crate) fn call(id: &str, tool: &str, call_id: &str, arguments: &str) -> Message {
    Message::tool_call(id, call_id, tool, arguments)
}

pub(crate) fn result(id: &str, tool: &str, call_id: &str, output: &str) -> Message {
    Message::tool_result(id, call_id, tool, output)
}

/// A result projected with only its call id.
pub(crate) fn unnamed_result(id: &str, call_id: &str, output: &str) -> Message {
    Message {
        tool_name: None,
        ..Message::tool_result(id, call_id, "", output)
    }
}

/// A result whose call id is `tc-<id>`, as the kernel's recent-zone tests
/// build them.
fn tool_output(id: &str, tool: &str, output: &str) -> Message {
    result(id, tool, &format!("tc-{id}"), output)
}

pub(crate) fn recent(preserve_recent_messages: usize) -> Config {
    let mut config = config();
    config.preserve_recent_messages = preserve_recent_messages;
    config
}

pub(crate) fn tools(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub(crate) fn fold(
    messages: &[Message],
    config: &Config,
    start: &str,
    end: &str,
    summary: &str,
) -> ApplyOutcome {
    let state = state_for(messages);
    apply(&[range(start, end, summary)], messages, &state, config)
}

pub(crate) fn last_block_covers(outcome: &ApplyOutcome, id: &str) -> bool {
    outcome
        .state
        .blocks
        .last()
        .expect("a block")
        .effective_message_ids
        .iter()
        .any(|covered| covered == id)
}

fn last_block_folds(outcome: &ApplyOutcome, id: &str) -> bool {
    outcome
        .state
        .blocks
        .last()
        .expect("a block")
        .direct_message_ids
        .iter()
        .any(|folded| folded == id)
}

pub(crate) fn joined(warnings: &[String]) -> String {
    warnings.join(" ")
}

// --- protect-recent ---

fn eight() -> Vec<Message> {
    [
        ("a", "first detailed message body content alpha"),
        ("b", "second detailed message body content beta"),
        ("c", "third detailed message body content gamma"),
        ("d", "fourth detailed message body content delta"),
        ("e", "fifth detailed message body content epsilon"),
        ("f", "sixth detailed message body content zeta"),
        ("g", "seventh detailed message body content eta"),
        ("h", "eighth detailed message body content theta"),
    ]
    .into_iter()
    .map(|(id, body)| text(id, body))
    .collect()
}

#[test]
fn a_range_entirely_inside_the_recent_zone_is_a_no_op() {
    let outcome = fold(
        &eight(),
        &recent(3),
        "m00006",
        "m00008",
        "trying to compress the recent zone",
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.errors.is_empty());
    assert!(outcome.result.noop);
    let warnings = joined(&outcome.result.warnings);
    assert!(warnings.contains("protected zone"), "{warnings}");
    assert!(warnings.contains("do not resend this range"), "{warnings}");
}

#[test]
fn the_protected_tail_is_excluded_and_the_rest_folds() {
    let outcome = fold(
        &eight(),
        &recent(3),
        "m00005",
        "m00007",
        "overlapping the recent zone",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
    assert_eq!(outcome.result.warnings.len(), 1);
    let warning = &outcome.result.warnings[0];
    assert!(
        warning.starts_with("Excluded") && warning.contains("m00006"),
        "{warning}"
    );
    assert!(!last_block_covers(&outcome, "f") && !last_block_covers(&outcome, "g"));
    assert!(last_block_folds(&outcome, "e"));
}

#[test]
fn the_most_recent_user_message_is_excluded_and_the_rest_folds() {
    let messages = [
        text("a", "u1 text alpha"),
        assistant("b", "assistant reply beta"),
        text("c", "u2 text gamma — the latest user message"),
    ];
    let outcome = fold(
        &messages,
        &recent(1),
        "m00002",
        "m00003",
        "grabbing the last user msg",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
    assert_eq!(outcome.result.warnings.len(), 1);
    assert!(outcome.result.warnings[0].contains("m00003"));
    assert!(last_block_folds(&outcome, "b"));
    assert!(!last_block_covers(&outcome, "c"));
}

#[test]
fn messages_strictly_before_the_zone_still_fold() {
    let outcome = fold(
        &eight(),
        &recent(3),
        "m00001",
        "m00003",
        "compressing older messages is allowed",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
    assert!(outcome.result.warnings.is_empty());
}

#[test]
fn the_zone_is_computed_when_no_set_is_given() {
    let messages = [
        text("a", "old user msg alpha"),
        assistant("b", "old assistant beta"),
        text("c", "current user intent gamma"),
    ];
    let outcome = fold(
        &messages,
        &recent(2),
        "m00002",
        "m00003",
        "should be refused by default protection",
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.errors.is_empty());
    assert!(outcome.result.noop);
    assert!(joined(&outcome.result.warnings).contains("protected zone"));
}

#[test]
fn decompress_results_do_not_count_toward_the_zone() {
    let messages = [
        text("a", "old alpha"),
        assistant("b", "old beta"),
        text("c", "old gamma"),
        tool_output("d", "decompress", &"x".repeat(20_000)),
    ];
    let state = state_for(&messages);
    let zone = compute_protected_refs(&messages, &state, &recent(3));
    assert!(!zone.contains("m00004"));
    assert!(
        zone.contains("m00003"),
        "the last user message stays protected"
    );
}

#[test]
fn a_decompress_result_in_the_tail_folds() {
    let messages = [
        text("a", "first message alpha"),
        assistant("b", "second message beta"),
        text("c", "third message gamma"),
        tool_output(
            "d",
            "decompress",
            &format!("restored content {}", "x".repeat(200)),
        ),
    ];
    let outcome = fold(
        &messages,
        &recent(3),
        "m00004",
        "m00004",
        "compressing the decompress result",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
    assert!(outcome.result.warnings.is_empty());
}

#[test]
fn warnings_accumulate_across_ranges() {
    let mut messages = eight();
    messages.push(text("i", "ninth message iota"));
    messages.push(assistant("j", "tenth message kappa"));
    let state = state_for(&messages);
    let outcome = apply(
        &[
            with_topic(
                range("m00004", "m00009", "first partial range summary"),
                "a",
            ),
            with_topic(
                range("m00008", "m00010", "second range overlaps the first"),
                "b",
            ),
        ],
        &messages,
        &state,
        &recent(3),
    );
    assert!(outcome.result.blocks_created >= 1);
    assert!(
        outcome.result.warnings.len() >= 2,
        "{:?}",
        outcome.result.warnings
    );
}

#[test]
fn never_preserve_recent_defaults_to_the_built_in_list() {
    for name in ["decompress", "search_context", "read", "bash"] {
        assert!(
            is_never_preserve_recent(&tool_output("x", name, "body"), None, &[]),
            "{name}"
        );
    }
    assert!(!is_never_preserve_recent(
        &tool_output("y", "grep", "hits"),
        None,
        &[]
    ));
    assert!(!is_never_preserve_recent(
        &text("z", "plain text"),
        None,
        &[]
    ));
}

#[test]
fn never_preserve_recent_honors_an_explicit_list() {
    let read = tool_output("a", "read", "file body");
    let bash = tool_output("b", "bash", "output");
    let read_file = tool_output("c", "read_file", "file body");
    assert!(is_never_preserve_recent(
        &bash,
        Some(&tools(&["bash"])),
        &[]
    ));
    assert!(!is_never_preserve_recent(
        &read,
        Some(&tools(&["bash"])),
        &[]
    ));
    assert!(!is_never_preserve_recent(&read, Some(&[]), &[]));
    assert!(!is_never_preserve_recent(&bash, Some(&[]), &[]));
    assert!(is_never_preserve_recent(
        &read_file,
        Some(&tools(&["read*"])),
        &[]
    ));
    assert!(is_never_preserve_recent(
        &read,
        Some(&tools(&["read*"])),
        &[]
    ));
}

#[test]
fn the_configured_list_flips_zone_membership() {
    let messages = [
        text("a", "old alpha"),
        assistant("b", "old beta"),
        text("c", "old gamma"),
        tool_output("d", "read", &"x".repeat(20_000)),
        text("e", "latest user intent"),
    ];
    let state = state_for(&messages);
    let zone_with = |list: Option<&[&str]>| {
        let mut config = recent(3);
        config.never_preserve_recent_tools = list.map(tools);
        compute_protected_refs(&messages, &state, &config)
    };
    assert!(!zone_with(None).contains("m00004"));
    assert!(zone_with(Some(&["bash"])).contains("m00004"));
    assert!(!zone_with(Some(&["read", "bash"])).contains("m00004"));
    assert!(zone_with(Some(&[])).contains("m00004"));
}

fn fresh_read() -> Vec<Message> {
    vec![
        text("a", "first message alpha"),
        assistant("b", "second message beta"),
        text("c", "third message gamma"),
        tool_output(
            "d",
            "read",
            &format!("freshly read file body {}", "x".repeat(200)),
        ),
        assistant("e", "fourth message epsilon"),
        text("f", "fifth message zeta"),
    ]
}

#[test]
fn an_empty_list_protects_a_fresh_read_and_the_default_reclaims_it() {
    let messages = fresh_read();
    let mut empty = recent(3);
    empty.never_preserve_recent_tools = Some(Vec::new());
    let refused = fold(
        &messages,
        &empty,
        "m00004",
        "m00004",
        "should be refused while in the recent zone",
    );
    assert_eq!(refused.result.blocks_created, 0);
    assert!(refused.result.errors.is_empty());
    assert!(refused.result.noop);
    assert!(joined(&refused.result.warnings).contains("protected zone"));

    let allowed = fold(
        &messages,
        &recent(3),
        "m00004",
        "m00004",
        "reclaiming the spent read result",
    );
    assert_eq!(allowed.result.blocks_created, 1);
    assert!(allowed.result.errors.is_empty());
}

#[test]
fn a_read_result_ages_out_of_the_zone() {
    let messages = [
        text("a", "first message alpha"),
        assistant("b", "second message beta"),
        text("c", "third message gamma"),
        tool_output("d", "read", &format!("spent file body {}", "x".repeat(200))),
        assistant("e", "fourth message epsilon"),
        text("f", "fifth message zeta"),
        assistant("g", "sixth message eta"),
        text("h", "seventh message theta"),
    ];
    let mut empty = recent(3);
    empty.never_preserve_recent_tools = Some(Vec::new());
    let outcome = fold(
        &messages,
        &empty,
        "m00004",
        "m00004",
        "aging out of the zone",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
}

fn zone_preserving(
    messages: &[Message],
    never: Option<&[&str]>,
    preserve: &[&str],
) -> HashSet<String> {
    let mut config = recent(3);
    config.never_preserve_recent_tools = never.map(tools);
    config.preserve_recent_tools = tools(preserve);
    compute_protected_refs(messages, &state_for(messages), &config)
}

#[test]
fn preserve_recent_tools_subtracts_from_the_built_in_list() {
    let messages = fresh_read();
    assert!(!zone_preserving(&messages, None, &[]).contains("m00004"));
    assert!(zone_preserving(&messages, None, &["read"]).contains("m00004"));

    let mut bash = messages.clone();
    bash[3] = tool_output("d", "bash", &format!("command output {}", "x".repeat(200)));
    assert!(
        !zone_preserving(&bash, None, &["read"]).contains("m00004"),
        "the subtraction is per tool"
    );
}

#[test]
fn preserve_recent_tools_combines_with_an_explicit_list() {
    let messages = [
        text("a", "first message alpha"),
        text("b", "second message beta"),
        text("c", "third message gamma"),
        tool_output(
            "d",
            "read",
            &format!("freshly read file body {}", "x".repeat(200)),
        ),
        tool_output("e", "bash", &format!("command output {}", "y".repeat(200))),
        text("f", "fifth message zeta"),
    ];
    let zone = zone_preserving(&messages, Some(&["read", "bash"]), &["read"]);
    assert!(zone.contains("m00004"));
    assert!(!zone.contains("m00005"));
}

#[test]
fn preserve_recent_tools_takes_globs_and_tolerates_no_match() {
    let messages = [
        text("a", "first message alpha"),
        text("b", "second message beta"),
        text("c", "third message gamma"),
        tool_output("d", "bash", &format!("command output {}", "x".repeat(200))),
        text("e", "fourth message epsilon"),
        text("f", "fifth message zeta"),
    ];
    assert!(zone_preserving(&messages, None, &["bash*"]).contains("m00004"));
    assert!(!zone_preserving(&messages, None, &["nonexistent_tool"]).contains("m00004"));
    assert!(!zone_preserving(&messages, None, &[]).contains("m00004"));
}

#[test]
fn preserve_recent_tools_read_keeps_a_fresh_read_from_folding() {
    let messages = fresh_read();
    let mut config = recent(3);
    config.preserve_recent_tools = tools(&["read"]);
    let refused = fold(
        &messages,
        &config,
        "m00004",
        "m00004",
        "fold the fresh read",
    );
    assert_eq!(refused.result.blocks_created, 0);
    assert!(refused.result.errors.is_empty());
    assert!(refused.result.noop);
    assert!(joined(&refused.result.warnings).contains("protected zone"));
}

#[test]
fn tool_patterns_ignore_case() {
    assert!(match_tool_pattern("read", "read"));
    assert!(match_tool_pattern("Read", "read"));
    assert!(match_tool_pattern("read", "READ"));
    assert!(match_tool_pattern("BASH", "bash"));
    assert!(!match_tool_pattern("Grep", "read"));
    assert!(match_tool_pattern("ReadFile", "read*"));
    assert!(match_tool_pattern("READ_FILE", "read*"));
    assert!(match_tool_pattern("search_context", "SEARCH*"));
    assert!(!match_tool_pattern("write_file", "read*"));
}

#[test]
fn capitalized_built_in_results_are_excluded_from_the_zone() {
    assert!(is_never_preserve_recent(
        &tool_output("a", "Read", "body"),
        None,
        &[]
    ));
    assert!(is_never_preserve_recent(
        &tool_output("b", "Bash", "out"),
        None,
        &[]
    ));
    assert!(is_never_preserve_recent(
        &tool_output("c", "DECOMPRESS", "x"),
        None,
        &[]
    ));
    assert!(!is_never_preserve_recent(
        &tool_output("d", "grep", "hits"),
        None,
        &[]
    ));
    assert!(!is_never_preserve_recent(
        &tool_output("a", "Read", "body"),
        None,
        &tools(&["READ"])
    ));

    let messages = [
        text("a", "old alpha"),
        assistant("b", "old beta"),
        text("c", "old gamma"),
        tool_output("d", "Read", &"x".repeat(20_000)),
        text("e", "latest user intent"),
    ];
    let zone = compute_protected_refs(&messages, &state_for(&messages), &recent(3));
    assert!(!zone.contains("m00004"));
}

// --- protected-content ---

pub(crate) const VALID_SUMMARY: &str = "A meaningful summary that captures the key information of the compressed range including file paths and decisions.";

pub(crate) fn long_text() -> String {
    "x".repeat(6000)
}

fn around(middle: [Message; 2]) -> Vec<Message> {
    let [call, result] = middle;
    vec![
        text("a", &long_text()),
        call,
        result,
        text("d", &long_text()),
    ]
}

pub(crate) fn protecting(names: &[&str]) -> Config {
    let mut config = config();
    config.protected_tools = tools(names);
    config
}

#[test]
fn a_protected_tool_call_and_its_result_stay_out_of_the_block() {
    let messages = around([
        call("b", "skill", "call1", r#"{"name":"git-master"}"#),
        unnamed_result("c", "call1", "skill output"),
    ]);
    let outcome = fold(
        &messages,
        &protecting(&["skill"]),
        "m00001",
        "m00004",
        VALID_SUMMARY,
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
    let block = &outcome.state.blocks[0];
    for id in ["b", "c"] {
        assert!(!block.direct_message_ids.contains(&id.to_owned()), "{id}");
        assert!(
            !block.effective_message_ids.contains(&id.to_owned()),
            "{id}"
        );
    }
    for id in ["a", "d"] {
        assert!(block.direct_message_ids.contains(&id.to_owned()), "{id}");
        assert!(block.effective_message_ids.contains(&id.to_owned()), "{id}");
    }
    assert_eq!(
        block.summary, VALID_SUMMARY,
        "the summary is exactly what the author wrote"
    );
}

#[test]
fn an_unprotected_tool_call_folds() {
    let messages = around([
        call("b", "bash", "call1", r#"{"command":"ls"}"#),
        unnamed_result("c", "call1", "file1\nfile2"),
    ]);
    let outcome = fold(
        &messages,
        &protecting(&["skill"]),
        "m00001",
        "m00004",
        VALID_SUMMARY,
    );
    let block = &outcome.state.blocks[0];
    assert!(block.direct_message_ids.contains(&"b".to_owned()));
    assert!(block.direct_message_ids.contains(&"c".to_owned()));
}

#[test]
fn a_wildcard_protected_tool_pattern_matches() {
    let messages = around([
        call("b", "skill-git-master", "call1", "{}"),
        unnamed_result("c", "call1", "done"),
    ]);
    let outcome = fold(
        &messages,
        &protecting(&["skill-*"]),
        "m00001",
        "m00004",
        VALID_SUMMARY,
    );
    assert!(!outcome.state.blocks[0]
        .direct_message_ids
        .contains(&"b".to_owned()));
}

#[test]
fn the_compress_tool_is_always_protected() {
    for (protected_tools, result_named) in
        [(vec!["compress"], true), (vec![], true), (vec![], false)]
    {
        let arguments = serde_json_args();
        let compress_result = if result_named {
            result("c", "compress", "call1", "compressed")
        } else {
            unnamed_result("c", "call1", &"result".repeat(50))
        };
        let messages = around([call("b", "compress", "call1", &arguments), compress_result]);
        let outcome = fold(
            &messages,
            &protecting(&protected_tools),
            "m00001",
            "m00004",
            VALID_SUMMARY,
        );
        let block = &outcome.state.blocks[0];
        assert!(!block.direct_message_ids.contains(&"b".to_owned()));
        assert!(!block.direct_message_ids.contains(&"c".to_owned()));
        assert!(block.direct_message_ids.contains(&"a".to_owned()));
        assert!(block.direct_message_ids.contains(&"d".to_owned()));
    }
}

fn serde_json_args() -> String {
    format!(
        r#"{{"content":[{{"startId":"m00001","endId":"m00001","summary":"{}"}}]}}"#,
        "x".repeat(60)
    )
}

// --- protected-latest ---

fn todo_call(id: &str, call_id: &str, revision: u32) -> Message {
    call(
        id,
        "todo_list",
        call_id,
        &format!(r#"{{"action":"update","revision":{revision}}}"#),
    )
}

fn todo_result(id: &str, call_id: &str, revision: u32) -> Message {
    unnamed_result(
        id,
        call_id,
        &format!(r#"{{"revision":{revision},"todos":[]}}"#),
    )
}

fn latest_tools(names: &[&str]) -> Config {
    let mut config = config();
    config.protected_latest_tools = tools(names);
    config
}

pub(crate) fn sorted(set: &HashSet<String>) -> Vec<String> {
    let mut items: Vec<String> = set.iter().cloned().collect();
    items.sort();
    items
}

#[test]
fn collect_latest_protected_picks_the_last_matching_call() {
    let pair = [todo_call("c1", "t1", 1), todo_result("r1", "t1", 1)];
    let none = collect_latest_protected(&pair, &config());
    assert!(none.call_ids.is_empty() && none.message_ids.is_empty());

    let messages = [
        todo_call("c1", "t1", 1),
        todo_result("r1", "t1", 1),
        text("u", "continue"),
        todo_call("c2", "t2", 2),
        todo_result("r2", "t2", 2),
    ];
    let latest = collect_latest_protected(&messages, &latest_tools(&["todo_list"]));
    assert_eq!(sorted(&latest.call_ids), ["t2"]);
    assert!(latest.message_ids.is_empty());

    let glob = collect_latest_protected(&[todo_call("c1", "t1", 1)], &latest_tools(&["todo_*"]));
    assert_eq!(sorted(&glob.call_ids), ["t1"]);

    let without_id = Message {
        tool_call_id: None,
        ..todo_call("c1", "t1", 1)
    };
    let by_message = collect_latest_protected(&[without_id], &latest_tools(&["todo_list"]));
    assert_eq!(sorted(&by_message.message_ids), ["c1"]);
    assert!(by_message.call_ids.is_empty());

    let unmatched = collect_latest_protected(&[text("u", "hi")], &latest_tools(&["todo_list"]));
    assert!(unmatched.call_ids.is_empty() && unmatched.message_ids.is_empty());
}

#[test]
fn only_the_latest_call_and_its_result_are_latest_protected() {
    let messages = [
        todo_call("c1", "t1", 1),
        todo_result("r1", "t1", 1),
        todo_call("c2", "t2", 2),
        todo_result("r2", "t2", 2),
    ];
    let latest = collect_latest_protected(&messages, &latest_tools(&["todo_list"]));
    assert!(is_message_latest_protected(&messages[2], &latest));
    assert!(is_message_latest_protected(&messages[3], &latest));
    assert!(!is_message_latest_protected(&messages[0], &latest));
    assert!(!is_message_latest_protected(&messages[1], &latest));
    assert!(!is_message_latest_protected(&text("u", "hi"), &latest));
}

#[test]
fn only_the_latest_pair_is_blocked() {
    let messages = [
        text("a", "start"),
        todo_call("c1", "t1", 1),
        todo_result("r1", "t1", 1),
        text("b", "middle"),
        todo_call("c2", "t2", 2),
        todo_result("r2", "t2", 2),
    ];
    let mut state = State::default();
    assign_message_refs(&mut state, &messages, &latest_tools(&["todo_list"]));
    let by_raw = &state.message_refs.by_raw;
    assert_eq!(by_raw["a"], "m00001");
    assert_eq!(by_raw["c1"], "m00002");
    assert_eq!(by_raw["r1"], "m00003");
    assert_eq!(by_raw["b"], "m00004");
    assert_eq!(by_raw["c2"], BLOCKED_REF);
    assert_eq!(by_raw["r2"], BLOCKED_REF);
}

fn todo_session(pairs: u32) -> Vec<Message> {
    let mut messages = Vec::new();
    for revision in 1..=pairs {
        messages.push(text(&format!("x{revision}"), &long_text()));
        messages.push(todo_call(
            &format!("c{revision}"),
            &format!("t{revision}"),
            revision,
        ));
        messages.push(todo_result(
            &format!("r{revision}"),
            &format!("t{revision}"),
            revision,
        ));
    }
    messages
}

#[test]
fn the_latest_pair_stays_out_and_older_pairs_fold() {
    let outcome = fold(
        &todo_session(2),
        &latest_tools(&["todo_list"]),
        "m00001",
        "m00006",
        VALID_SUMMARY,
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
    let block = &outcome.state.blocks[0];
    for id in ["c1", "r1"] {
        assert!(block.direct_message_ids.contains(&id.to_owned()), "{id}");
        assert!(block.effective_message_ids.contains(&id.to_owned()), "{id}");
    }
    for id in ["c2", "r2"] {
        assert!(!block.direct_message_ids.contains(&id.to_owned()), "{id}");
        assert!(
            !block.effective_message_ids.contains(&id.to_owned()),
            "{id}"
        );
    }
}

#[test]
fn a_superseded_pair_folds_once_a_newer_pair_exists() {
    let outcome = fold(
        &todo_session(3),
        &latest_tools(&["todo_list"]),
        "m00001",
        "m00006",
        VALID_SUMMARY,
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = &outcome.state.blocks[0];
    for id in ["c1", "r1", "c2", "r2"] {
        assert!(block.direct_message_ids.contains(&id.to_owned()), "{id}");
    }
    for id in ["c3", "r3"] {
        assert!(!block.direct_message_ids.contains(&id.to_owned()), "{id}");
    }
}

#[test]
fn hard_protection_excludes_every_pair() {
    let outcome = fold(
        &todo_session(2),
        &protecting(&["todo_list"]),
        "m00001",
        "m00006",
        VALID_SUMMARY,
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = &outcome.state.blocks[0];
    for id in ["c1", "r1", "c2", "r2"] {
        assert!(!block.direct_message_ids.contains(&id.to_owned()), "{id}");
    }
}

// --- protected-paths (the name-only and `a/*` forms) ---

#[test]
fn tool_paths_match_the_three_pattern_forms() {
    assert!(match_tool_path("skill/review-loop", "skill"));
    assert!(match_tool_path("skill", "skill"));
    assert!(!match_tool_path("read", "skill"));
    assert!(match_tool_path("skill/review-loop", "skill/*"));
    assert!(match_tool_path("skill", "skill/*"));
    assert!(!match_tool_path("read", "skill/*"));
    assert!(match_tool_path(
        "skill/release-orchestrator",
        "skill/release-orchestrator"
    ));
    assert!(!match_tool_path(
        "skill/release-orchestrator-v2",
        "skill/release-orchestrator"
    ));
    assert!(match_tool_path("skill/review-loop", "skill/review-*"));
    assert!(match_tool_path("skill/review", "skill/review*"));
    assert!(!match_tool_path("skill/release", "skill/review-*"));
    assert!(!match_tool_path("skill", "skill/review-*"));
    assert!(match_tool_path("skill/Review-Loop", "SKILL/review-loop"));
    assert!(match_tool_path("skill/review-loop", "SKILL/*"));
    assert!(!match_tool_path("skill/review-loop", "skill/review-loop/x"));
}

#[test]
fn a_skill_call_matches_a_bare_skill_pattern() {
    let skill = call("d", "skill", "c4", "no json");
    assert!(is_message_protected(
        &skill,
        &protecting(&["skill"]),
        &HashSet::new()
    ));
}

#[test]
fn a_plain_latest_pattern_keeps_one_latest_call() {
    let messages = [
        call("a", "skill", "c1", r#"{"name":"review-loop"}"#),
        call("b", "skill", "c2", r#"{"name":"audit-checklist"}"#),
    ];
    let latest = collect_latest_protected(&messages, &latest_tools(&["skill"]));
    assert!(!latest.call_ids.contains("c1"));
    assert!(latest.call_ids.contains("c2"));
}
