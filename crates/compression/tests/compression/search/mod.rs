// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `search.test.ts`, `doc-cache.test.ts` and the search cases of
// `truncate-surrogates.test.ts`. Not ported: the algorithm registry, the
// semantic and asynchronous search (not ported, plan §9), the V8 argument
// limit at 160K documents, and the cases that need a dictionary segmenter
// (listed where they would be). This module holds the shared helpers and
// the documents' and results' shapes.

mod cache;
mod quality;
mod tokenizer;

use hanafubuki_agent_compression::{
    block_docs, folded_message_inputs, message_docs, search_blocks, Algorithm, Block, DocCache,
    MessageInput, Role, SearchDoc, SearchDocKind, SearchOptions, SearchResult, State,
};

use crate::fixtures::{apply, assistant, config, range, state_for, user};

const EMOJI: &str = "\u{1F980}";

pub(crate) fn block(block_id: &str, topic: &str, summary: &str) -> Block {
    Block {
        block_id: block_id.to_owned(),
        run_id: "r1".to_owned(),
        topic: Some(topic.to_owned()),
        summary: summary.to_owned(),
        ..Block::default()
    }
}

pub(crate) fn blocks(blocks: Vec<Block>) -> Vec<SearchDoc> {
    block_docs(&State {
        blocks,
        ..State::default()
    })
}

pub(crate) fn message(r#ref: &str, role: Role, text: &str, block_id: &str) -> MessageInput {
    MessageInput {
        r#ref: r#ref.to_owned(),
        role,
        text: text.to_owned(),
        tokens: None,
        block_id: Some(block_id.to_owned()),
        tier: None,
    }
}

pub(crate) fn search(docs: &[SearchDoc], query: &str) -> Vec<SearchResult> {
    search_with(docs, query, SearchOptions::default())
}

pub(crate) fn search_with(
    docs: &[SearchDoc],
    query: &str,
    options: SearchOptions,
) -> Vec<SearchResult> {
    search_blocks(docs, query, &options, &mut DocCache::default())
}

pub(crate) fn with_algorithm(algorithm: Algorithm) -> SearchOptions {
    SearchOptions {
        algorithm,
        ..SearchOptions::default()
    }
}

pub(crate) fn refs(results: &[SearchResult]) -> Vec<&str> {
    results.iter().map(|result| result.r#ref.as_str()).collect()
}

#[test]
fn block_docs_include_active_and_inactive_blocks() {
    let mut archived = block("b2", "archived", "old content");
    archived.active = false;
    let docs = blocks(vec![block("b1", "active", "live content"), archived]);
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].text, "active live content");
    assert_eq!(docs[1].r#ref, "b2");
}

#[test]
fn a_match_in_an_active_block_is_found() {
    let docs = blocks(vec![
        block("b1", "Auth", "Auth token refresh"),
        block("b2", "DB", "database pool"),
    ]);
    let results = search(&docs, "auth");
    assert_eq!(refs(&results), ["b1"]);
    assert_eq!(results[0].kind, SearchDocKind::Block);
    assert!(results[0].score > 0.0);
}

#[test]
fn a_match_in_an_inactive_block_is_found() {
    let mut old = block("b2", "", "old auth token logic");
    old.active = false;
    let docs = blocks(vec![block("b1", "", "current work"), old]);
    assert_eq!(refs(&search(&docs, "auth")), ["b2"]);
}

#[test]
fn message_docs_carry_role_and_owning_block() {
    let docs = message_docs(&[
        message("m00100", Role::User, "how does auth work", "b1"),
        message("m00200", Role::Tool, "auth.ts: 401 handler", "b1"),
    ]);
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].kind, SearchDocKind::Message);
    assert_eq!(docs[0].role, Some(Role::User));
    assert_eq!(docs[0].block_id.as_deref(), Some("b1"));
    assert_eq!(docs[0].title, "user: how does auth work");
}

#[test]
fn messages_are_searched_by_their_original_text() {
    let docs = message_docs(&[
        message("m00100", Role::User, "implement jwt refresh endpoint", "b1"),
        message("m00200", Role::Tool, "postgres connection string", "b2"),
    ]);
    let results = search(&docs, "jwt");
    assert_eq!(refs(&results), ["m00100"]);
    assert_eq!(results[0].kind, SearchDocKind::Message);
    assert_eq!(results[0].block_id.as_deref(), Some("b1"));
}

#[test]
fn a_user_message_outranks_a_tool_message_with_the_same_text() {
    let docs = message_docs(&[
        message("m-tool", Role::Tool, "match match here", "b1"),
        message("m-user", Role::User, "match match here", "b1"),
    ]);
    let results = search(&docs, "match");
    assert_eq!(results[0].r#ref, "m-user");
    assert!(results[0].score > results[1].score);
}

#[test]
fn equal_role_weights_tie_the_scores() {
    let docs = message_docs(&[
        message("m-tool", Role::Tool, "match match here", "b1"),
        message("m-user", Role::User, "match match here", "b1"),
    ]);
    let mut options = SearchOptions::default();
    options.role_weights.user = 1.0;
    options.role_weights.tool = 1.0;
    let results = search_with(&docs, "match", options);
    assert!((results[0].score - results[1].score).abs() < 1e-9);
    assert_eq!(
        refs(&results),
        ["m-tool", "m-user"],
        "a tie keeps the order"
    );
}

#[test]
fn blocks_and_messages_rank_together() {
    let mut docs = blocks(vec![block("b1", "auth", "auth token refresh")]);
    docs.extend(message_docs(&[message(
        "m00500",
        Role::Assistant,
        "auth token refresh detail",
        "b1",
    )]));
    let results = search(&docs, "auth");
    assert_eq!(results.len(), 2);
    let found = refs(&results);
    assert!(found.contains(&"b1") && found.contains(&"m00500"));
}

#[test]
fn a_message_result_carries_tokens_and_its_block() {
    let mut input = message(
        "m00420",
        Role::User,
        &format!("{:.<500}", "login flow design"),
        "b7",
    );
    input.tokens = Some(150);
    let results = search(&message_docs(&[input]), "login");
    assert_eq!(results[0].r#ref, "m00420");
    assert_eq!(results[0].block_id.as_deref(), Some("b7"));
    assert_eq!(results[0].tokens, Some(150));
    assert!(results[0].preview.contains("login"));
}

#[test]
fn the_preview_centers_on_the_matched_term() {
    let text = format!(
        "{} NEEDLE found here {}",
        "padding ".repeat(20),
        "more ".repeat(20)
    );
    let docs = message_docs(&[message("m1", Role::Assistant, &text, "b1")]);
    let options = SearchOptions {
        preview_length: 40,
        ..SearchOptions::default()
    };
    let preview = &search_with(&docs, "NEEDLE", options)[0].preview;
    assert!(preview.contains("NEEDLE"));
    assert!(!preview.starts_with("padding padding"));
}

#[test]
fn the_preview_anchors_on_the_first_term_found_only() {
    let text = format!(
        "{} cache strategy {} redis config",
        "a".repeat(20),
        "b".repeat(40)
    );
    let docs = message_docs(&[message("m1", Role::Assistant, &text, "b1")]);
    let options = SearchOptions {
        preview_length: 40,
        ..SearchOptions::default()
    };
    let preview = &search_with(&docs, "cache redis", options)[0].preview;
    assert!(preview.contains("cache"), "{preview}");
    assert!(!preview.contains("redis"), "{preview}");
}

#[test]
fn an_empty_query_finds_nothing() {
    let docs = blocks(vec![block("b1", "", "content")]);
    assert!(search(&docs, "").is_empty());
    assert!(search(&docs, "   ").is_empty());
}

#[test]
fn the_limit_is_respected() {
    let docs = blocks(
        (0..20)
            .map(|index| block(&format!("b{index}"), "", "match match match"))
            .collect(),
    );
    let options = SearchOptions {
        limit: 5,
        ..SearchOptions::default()
    };
    assert_eq!(search_with(&docs, "match", options).len(), 5);
}

#[test]
fn results_are_sorted_by_score() {
    let docs = blocks(vec![
        block("b1", "", "one match"),
        block("b2", "", "match match match match"),
        block("b3", "", "match match"),
    ]);
    let results = search(&docs, "match");
    assert_eq!(results[0].r#ref, "b2");
    assert!(results[0].score >= results[1].score);
}

#[test]
fn search_ignores_case() {
    let docs = blocks(vec![block("b1", "", "Auth Token Refresh")]);
    assert_eq!(search(&docs, "AUTH token").len(), 1);
}

#[test]
fn the_algorithm_option_selects_the_scorer() {
    let docs = blocks(vec![block("b1", "", "auth token")]);
    let hybrid = search_with(&docs, "auth", with_algorithm(Algorithm::Hybrid));
    let substring = search_with(&docs, "auth", with_algorithm(Algorithm::Substring));
    assert!(substring[0].score >= 1.0, "substring counts occurrences");
    assert!(hybrid[0].score <= 1.0001, "hybrid is normalized");
}

#[test]
fn a_title_cut_through_an_astral_character_drops_it() {
    let text = format!("{}{EMOJI} needle details", "h".repeat(59));
    let docs = message_docs(&[MessageInput {
        block_id: None,
        ..message("m00001", Role::User, &text, "")
    }]);
    let results = search(&docs, "needle");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, format!("user: {}", "h".repeat(59)));
}

#[test]
fn a_preview_window_through_astral_characters_drops_them() {
    let text = format!(
        "{}{EMOJI}{}needle{}{EMOJI}{}",
        "x".repeat(19),
        "y".repeat(9),
        "w".repeat(23),
        "v".repeat(20)
    );
    let docs = vec![SearchDoc {
        kind: SearchDocKind::Block,
        r#ref: "b1".to_owned(),
        text,
        title: "t".to_owned(),
        role: None,
        block_id: Some("b1".to_owned()),
        tier: Some(1),
        tokens: None,
    }];
    let options = SearchOptions {
        preview_length: 40,
        ..SearchOptions::default()
    };
    let preview = &search_with(&docs, "needle", options)[0].preview;
    assert_eq!(
        *preview,
        format!("…{}needle{}…", "y".repeat(9), "w".repeat(23))
    );
}

#[test]
fn folded_messages_are_searchable_with_the_block_that_holds_them() {
    let messages = [
        user("u1", "implement jwt refresh endpoint"),
        assistant("a1", "done"),
        user("u2", "visible request about jwt"),
    ];
    let outcome = apply(
        &[range("m00001", "m00002", "an endpoint was added")],
        &messages,
        &state_for(&messages),
        &config(),
    );
    let inputs = folded_message_inputs(&outcome.state, &messages);
    let found: Vec<(&str, Option<&str>)> = inputs
        .iter()
        .map(|input| (input.r#ref.as_str(), input.block_id.as_deref()))
        .collect();
    assert_eq!(found, [("m00001", Some("b1")), ("m00002", Some("b1"))]);

    let mut docs = block_docs(&outcome.state);
    docs.extend(message_docs(&inputs));
    let results = search(&docs, "jwt");
    assert_eq!(refs(&results), ["m00001"]);
    assert_eq!(results[0].block_id.as_deref(), Some("b1"));
}
