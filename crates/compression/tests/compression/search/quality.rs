// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// The hybrid's quality properties and the kernel's pinned status quo, from
// `search.test.ts`. Without a dictionary a CJK word scores through BM25 as
// a bigram, so the `缓存` rescue scores past the fuzzy-only cap, and
// `试验证明` against `验证` (which needs word boundaries) is not ported.

use hanafubuki_agent_compression::{message_docs, Algorithm, Role};

use super::{block, blocks, message, refs, search, search_with, with_algorithm};

#[test]
fn a_cjk_query_matches_cjk_content() {
    let mut docs = blocks(vec![block("b1", "用户认证", "实现了用户登录认证流程")]);
    docs.extend(message_docs(&[message(
        "m1",
        Role::Assistant,
        "database postgres pool",
        "b2",
    )]));
    assert_eq!(search(&docs, "登录")[0].r#ref, "b1");
}

#[test]
fn fuzzy_tolerates_a_typo() {
    let docs = message_docs(&[message(
        "m1",
        Role::Assistant,
        "authentication token refresh",
        "b1",
    )]);
    assert_eq!(search(&docs, "tokan")[0].r#ref, "m1");
}

#[test]
fn stemming_matches_a_morphological_variant() {
    let docs = blocks(vec![
        block("b1", "compress", "the compress utility"),
        block("b2", "other", "completely different caching topic"),
    ]);
    assert_eq!(search(&docs, "compressed")[0].r#ref, "b1");
}

#[test]
fn a_whole_word_query_hits_the_document_holding_it() {
    let docs = blocks(vec![
        block("b1", "i18n", "i18n 国际化 setup with locales"),
        block("b2", "logs", "ELK 日志栈 采集 过滤 存储。结构化日志 JSON。"),
    ]);
    assert_eq!(search(&docs, "国际化")[0].r#ref, "b1");
}

#[test]
fn an_unknown_word_is_still_found() {
    let docs = blocks(vec![
        block("b1", "dash", "可视化"),
        block("b2", "cache", "缓存策略 redis 层"),
    ]);
    assert_eq!(search(&docs, "可视化")[0].r#ref, "b1");
}

#[test]
fn a_cjk_phrase_ranks_the_whole_match_first() {
    let docs = blocks(vec![
        block("b1", "dash", "可视化"),
        block("b2", "charts", "图表可视化"),
    ]);
    let results = search(&docs, "图表可视化");
    assert_eq!(results[0].r#ref, "b2");
    assert!(results[0].score > 0.5, "{}", results[0].score);
}

#[test]
fn the_authenticate_family_is_rescued_only_by_fuzzy() {
    let docs = blocks(vec![block("b1", "", "authenticate the user account")]);
    let results = search(&docs, "authentication");
    assert_eq!(results[0].r#ref, "b1");
    assert!(results[0].score < 0.7, "{}", results[0].score);
}

#[test]
fn a_two_character_cjk_query_reaches_the_fuzzy_scorer() {
    let docs = blocks(vec![block("b1", "", "缓存策略 redis 层")]);
    let results = search_with(&docs, "缓存", with_algorithm(Algorithm::Fuzzy));
    assert_eq!(refs(&results), ["b1"]);
    assert_eq!(results[0].score, 1.0);
}

#[test]
fn one_cjk_character_and_short_latin_never_reach_fuzzy() {
    let docs = blocks(vec![block("b1", "", "缓存策略 redis 层")]);
    for query in ["验", "ok", "to"] {
        assert!(
            search_with(&docs, query, with_algorithm(Algorithm::Fuzzy)).is_empty(),
            "{query}"
        );
    }
}

#[test]
fn a_two_character_cjk_typo_finds_nothing() {
    let docs = blocks(vec![block("b1", "用户认证", "实现了用户登录认证流程")]);
    assert!(search(&docs, "登入").is_empty());
}

#[test]
fn a_two_character_cjk_word_finds_its_block() {
    let docs = blocks(vec![
        block("b1", "viz", "可视化 dashboards"),
        block("b2", "cache", "缓存策略 redis 层"),
    ]);
    assert_eq!(refs(&search(&docs, "缓存")), ["b2"]);
}

#[test]
fn one_character_finds_a_one_character_run_and_no_word() {
    let docs = blocks(vec![
        block("b1", "auth", "身份验证流程"),
        block("b2", "viz", "可视化"),
    ]);
    assert!(search(&docs, "验")
        .iter()
        .all(|result| result.r#ref != "b1"));
    let single = blocks(vec![block("b3", "", "验")]);
    assert_eq!(search(&single, "验")[0].r#ref, "b3");
}

#[test]
fn a_split_camel_case_query_is_rescued_by_fuzzy() {
    let docs = blocks(vec![block("b1", "", "syncBlocks registry walk")]);
    let results = search(&docs, "sync blocks");
    assert_eq!(results[0].r#ref, "b1");
    assert!(results[0].score < 0.7, "{}", results[0].score);
}

#[test]
fn a_fuzzy_only_full_overlap_caps_at_the_fuzzy_weight() {
    let docs = message_docs(&[
        message("m1", Role::Assistant, "auth token refresh", "b1"),
        message("m2", Role::Assistant, "tokanxyz", "b2"),
    ]);
    let results = search(&docs, "tokan");
    assert_eq!(results[0].r#ref, "m2");
    assert!(results[0].score <= 0.31, "{}", results[0].score);
}

#[test]
fn an_exact_bm25_hit_beats_a_fuzzy_only_overlap() {
    let docs = message_docs(&[
        message("m1", Role::Assistant, "tokenized", "b1"),
        message("m2", Role::Assistant, "token", "b2"),
    ]);
    let results = search(&docs, "token");
    assert_eq!(results[0].r#ref, "m2");
    assert!(results[0].score >= 0.7, "{}", results[0].score);
}
