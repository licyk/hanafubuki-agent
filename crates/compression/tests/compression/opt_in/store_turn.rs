// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the arrival, per-request and fold-time cases of `ccr.test.ts`. Not
// ported: the legacy `acp_retrieved_` injections (retrieval is a tool result
// here, so they never exist), a request whose history is the previous
// request's rendered view (the host always passes its own history), and
// the status report's store counters (the report has none).

use hanafubuki_agent_compression::{
    is_absorb_candidate, is_stored_placeholder_text, note_retrieval, store_covered_originals,
    store_large_results, Config, ContentStore, ContentStoreConfig, Message, State, StoreSpec,
    BLOCKED_REF,
};

use super::store::{big_text, doc_with_embedded_example, source_defining_marker};
use super::turn_with;
use crate::fixtures::{apply, assistant, call, range, result, state_for, user};

fn tool_call(id: &str, call_id: &str, tool: &str) -> Message {
    call(
        id,
        call_id,
        tool,
        &format!(r#"{{"command":"run {call_id}"}}"#),
    )
}

fn storing() -> Config {
    let mut config = Config::new(100_000);
    config.content_store = ContentStoreConfig {
        enabled: true,
        ..ContentStoreConfig::default()
    };
    config
}

fn pair(text: &str) -> Vec<Message> {
    vec![
        tool_call("c1", "call1", "bash"),
        result("r1", "call1", "bash", text),
    ]
}

fn session() -> Vec<Message> {
    vec![
        user("u1", "build the project"),
        assistant("a1", "running the build now"),
        tool_call("c1", "call1", "bash"),
        result("r1", "call1", "bash", &big_text()),
    ]
}

#[test]
fn off_leaves_everything() {
    let messages = pair(&big_text());
    let mut store = ContentStore::default();
    let out = store_large_results(
        messages.clone(),
        &state_for(&messages),
        &mut store,
        &Config::new(100_000),
    );
    assert_eq!(out.stored_count, 0);
    assert_eq!(out.messages, messages);
}

#[test]
fn small_results_are_left_alone() {
    let messages = pair("short output");
    let mut store = ContentStore::default();
    let out = store_large_results(
        messages.clone(),
        &state_for(&messages),
        &mut store,
        &storing(),
    );
    assert_eq!(out.stored_count, 0);
    assert_eq!(out.messages, messages);
}

#[test]
fn a_large_result_is_stored_and_replaced_once() {
    let messages = pair(&big_text());
    let mut store = ContentStore::default();
    let out = store_large_results(
        messages.clone(),
        &state_for(&messages),
        &mut store,
        &storing(),
    );
    assert_eq!(out.stored_count, 1);
    assert_eq!(
        out.messages[1].text,
        "📦 [acp-stored #m00002 · shell output · 7,500 tok] `run call1`\n   → acp_retrieve(\"m00002\") returns the full text"
    );
    assert_eq!(out.messages[1].tool_call_id.as_deref(), Some("call1"));
    assert_eq!(out.messages[0], messages[0]);
    assert_eq!(store.retrieve("m00002").unwrap().1, big_text());
}

#[test]
fn a_replaced_result_is_not_stored_again() {
    let messages = pair(&big_text());
    let state = state_for(&messages);
    let mut store = ContentStore::default();
    let first = store_large_results(messages, &state, &mut store, &storing());
    let stored = store.clone();
    let second = store_large_results(first.messages.clone(), &state, &mut store, &storing());
    assert_eq!(second.stored_count, 0);
    assert_eq!(second.messages, first.messages);
    assert_eq!(store, stored);
}

#[test]
fn raw_bytes_sent_again_show_the_arrival_placeholder() {
    let messages = pair(&big_text());
    let state = state_for(&messages);
    let mut store = ContentStore::default();
    let first = store_large_results(messages.clone(), &state, &mut store, &storing());
    assert_eq!(store.by_ref["m00002"].command.as_deref(), Some("run call1"));
    let stored = store.clone();
    let again = store_large_results(messages, &state, &mut store, &storing());
    assert_eq!(again.stored_count, 0);
    assert_eq!(again.messages[1].text, first.messages[1].text);
    assert_eq!(store, stored);
    let partial = store_large_results(pair("partial output"), &state, &mut store, &storing());
    assert_eq!(
        partial.messages[1].text, first.messages[1].text,
        "even below the threshold"
    );
}

#[test]
fn a_turn_keeps_the_placeholder_byte_stable() {
    let (first, store) = turn_with(
        &session(),
        State::default(),
        &storing(),
        1000,
        ContentStore::default(),
        &[],
    );
    let placeholder = first.messages[3].text.clone();
    assert!(is_stored_placeholder_text(&placeholder));
    assert!(placeholder.contains("#m00004"));
    assert_eq!(first.state.stats.stored_count, 1);
    assert_eq!(store.retrieve("m00004").unwrap().1, big_text());
    let mut messages = session();
    messages.push(user("u2", "did it work?"));
    let (second, again) = turn_with(&messages, first.state, &storing(), 1200, store.clone(), &[]);
    assert_eq!(second.messages[3].text, placeholder);
    assert_eq!(second.state.stats.stored_count, 1);
    assert_eq!(again, store);
}

#[test]
fn off_the_store_comes_back_empty() {
    let (out, store) = turn_with(
        &session(),
        State::default(),
        &Config::new(100_000),
        1000,
        ContentStore::default(),
        &[],
    );
    assert_eq!(store, ContentStore::default());
    assert_eq!(out.messages[3].text, big_text());
    assert_eq!(out.state.stats.stored_count, 0);
}

#[test]
fn a_legacy_entry_takes_its_command_from_the_call_then_the_head() {
    let messages = pair(&big_text());
    let state = state_for(&messages);
    let mut legacy = ContentStore::default();
    legacy.store_original(StoreSpec {
        r#ref: "m00002".to_owned(),
        raw_id: "r1".to_owned(),
        text: big_text(),
        kind: "shell output".to_owned(),
        tool_name: Some("bash".to_owned()),
        tokens: 7500,
        head: "line of output line of output".to_owned(),
        command: None,
    });
    let with_arguments = store_large_results(messages, &state, &mut legacy, &storing());
    assert!(with_arguments.messages[1].text.contains("] `run call1`\n"));
    let without = vec![
        call("c1", "call1", "bash", "not json {"),
        result("r1", "call1", "bash", &big_text()),
    ];
    let without = store_large_results(without, &state, &mut legacy, &storing());
    assert!(without.messages[1]
        .text
        .contains("] `line of output line of output`\n"));
}

#[test]
fn acp_retrieve_excluded_and_protected_tools_are_skipped() {
    let mut messages = Vec::new();
    for (id, tool) in [
        ("compress", "compress"),
        ("retrieve", "acp_retrieve"),
        ("excluded", "secret_tool"),
        ("protected", "vault_tool"),
    ] {
        messages.push(tool_call(&format!("c-{id}"), &format!("call-{id}"), tool));
        messages.push(result(
            &format!("r-{id}"),
            &format!("call-{id}"),
            tool,
            &big_text(),
        ));
    }
    let state = state_for(&messages);
    let mut excluding = storing();
    excluding.content_store.exclude_tools = vec!["secret_*".to_owned(), "vault_*".to_owned()];
    let mut protecting = storing();
    protecting.content_store.exclude_tools = vec!["secret_*".to_owned()];
    protecting.protected_tools = vec!["vault_tool".to_owned()];
    for config in [excluding, protecting] {
        let out = store_large_results(
            messages.clone(),
            &state,
            &mut ContentStore::default(),
            &config,
        );
        assert_eq!(out.stored_count, 0);
    }
}

#[test]
fn results_without_a_call_id_or_with_a_blocked_ref_are_skipped() {
    let orphan = Message {
        tool_call_id: None,
        ..result("r1", "", "bash", &big_text())
    };
    let out = store_large_results(
        vec![orphan.clone()],
        &state_for(&[orphan]),
        &mut ContentStore::default(),
        &storing(),
    );
    assert_eq!(out.stored_count, 0);
    let blocked = result("r2", "call2", "bash", &big_text());
    let mut state = State::default();
    state
        .message_refs
        .by_raw
        .insert("r2".to_owned(), BLOCKED_REF.to_owned());
    let out = store_large_results(
        vec![blocked],
        &state,
        &mut ContentStore::default(),
        &storing(),
    );
    assert_eq!(out.stored_count, 0);
}

#[test]
fn ordinary_texts_quoting_a_placeholder_are_stored() {
    for original in [doc_with_embedded_example(), source_defining_marker()] {
        let messages = vec![
            tool_call("c1", "call1", "read"),
            result("r1", "call1", "read", &original),
        ];
        let mut store = ContentStore::default();
        let out = store_large_results(
            messages.clone(),
            &state_for(&messages),
            &mut store,
            &storing(),
        );
        assert_eq!(out.stored_count, 1);
        assert!(is_stored_placeholder_text(&out.messages[1].text));
        assert_eq!(store.retrieve("m00002").unwrap().1, original);
    }
}

#[test]
fn a_placeholder_is_not_an_absorb_candidate() {
    let mut config = Config::new(100_000);
    config.absorb.enabled = true;
    let messages = pair(&big_text());
    let out = store_large_results(
        messages.clone(),
        &state_for(&messages),
        &mut ContentStore::default(),
        &storing(),
    );
    assert!(!is_absorb_candidate(&out.messages[1], &config));
    assert!(is_absorb_candidate(&messages[1], &config));
    for text in [doc_with_embedded_example(), source_defining_marker()] {
        assert!(is_absorb_candidate(
            &result("r1", "call1", "bash", &text),
            &config
        ));
    }
}

#[test]
fn a_retrieval_is_counted() {
    let mut state = State::default();
    note_retrieval(&mut state);
    note_retrieval(&mut state);
    assert_eq!(state.stats.retrieval_count, 2);
}

// --- fold-time archiving (`storeCoveredOriginals`) ---

fn fold(messages: &[Message], start: &str, end: &str, store: &mut ContentStore) -> State {
    let mut config = Config::new(100_000);
    config.compress.min_compress_range = 0;
    config.preserve_recent_messages = 1;
    config.preserve_recent_tokens = 0;
    let state = state_for(messages);
    let folded = apply(
        &[range(start, end, &"S".repeat(60))],
        messages,
        &state,
        &config,
    );
    assert_eq!(
        folded.result.blocks_created, 1,
        "{:?}",
        folded.result.errors
    );
    let blocks: Vec<String> = folded
        .state
        .active_blocks()
        .map(|block| block.block_id.clone())
        .collect();
    store_covered_originals(store, messages, &folded.state, &blocks, 96);
    folded.state
}

fn tail() -> [Message; 2] {
    [
        user("u9", "recent tail user message"),
        assistant("a9", "recent tail assistant reply"),
    ]
}

#[test]
fn folded_originals_can_be_retrieved() {
    let mut messages = vec![
        user("u1", "first user message here"),
        assistant("a1", "first assistant reply here"),
        user("u2", "second user message here"),
        assistant("a2", "second assistant reply here"),
    ];
    messages.extend(tail());
    let mut store = ContentStore::default();
    fold(&messages, "m00001", "m00004", &mut store);
    assert_eq!(
        store.retrieve("m00001").unwrap().1,
        "first user message here"
    );
    assert!(!store.has_ref("m00005"));
}

#[test]
fn a_folded_tool_result_keeps_its_command() {
    let mut messages = pair(&big_text());
    messages.push(user("u2", "recent tail user message"));
    let mut store = ContentStore::default();
    fold(&messages, "m00001", "m00002", &mut store);
    let (entry, text) = store.retrieve("m00002").unwrap();
    assert_eq!(text, big_text());
    assert_eq!(entry.command.as_deref(), Some("run call1"));
}

#[test]
fn reasoning_is_never_archived_and_arrival_entries_win() {
    let mut messages = pair(&big_text());
    messages.push(Message::reasoning("th1", "private thinking"));
    messages.push(user("u2", "follow up question"));
    messages.extend(tail());
    let state = state_for(&messages);
    let mut store = ContentStore::default();
    store_large_results(messages.clone(), &state, &mut store, &storing());
    let arrival = store.by_ref["m00002"].clone();
    fold(&messages, "m00001", "m00004", &mut store);
    assert_eq!(store.by_ref["m00002"], arrival);
    assert!(!store.has_ref("m00003"));
}

#[test]
fn a_messages_own_placeholder_is_never_archived() {
    let own = super::store::placeholder_for("m00001");
    let mut messages = vec![
        user("u1", &own),
        assistant("a1", "assistant reply that folds alongside"),
    ];
    messages.extend(tail());
    let mut store = ContentStore::default();
    fold(&messages, "m00001", "m00002", &mut store);
    assert!(
        !store.has_ref("m00001"),
        "a lost original is missed honestly"
    );
    assert_eq!(
        store.retrieve("m00002").unwrap().1,
        "assistant reply that folds alongside"
    );
}

#[test]
fn ordinary_and_foreign_placeholder_texts_are_archived() {
    for original in [
        doc_with_embedded_example(),
        super::store::placeholder_for("m00099"),
    ] {
        let mut messages = vec![
            user("u1", &original),
            assistant("a1", "assistant reply that folds alongside"),
        ];
        messages.extend(tail());
        let mut store = ContentStore::default();
        fold(&messages, "m00001", "m00002", &mut store);
        assert_eq!(store.retrieve("m00001").unwrap().1, original);
        assert!(!store.has_ref("m00099"), "a cited ref gains no entry");
    }
}
