// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// The tokenizer and stemmer cases of `search.test.ts` and
// `doc-cache.test.ts`. A CJK run is its bigrams here, so the cases pinning
// the dictionary's words (`身份验证流程` as three words, Japanese and Korean
// words, `㐀㐁㐂`'s fallback singles) become the bigram contract below.

use hanafubuki_agent_compression::{char_bigrams, stem_word, term_frequencies, tokenize};

#[test]
fn a_cjk_run_becomes_its_bigrams_without_single_characters() {
    assert_eq!(
        tokenize("身份验证流程", false),
        ["身份", "份验", "验证", "证流", "流程"]
    );
    assert!(tokenize("可视化", false).contains(&"可视".to_owned()));
    assert!(!tokenize("身份验证流程", false).contains(&"份".to_owned()));
}

#[test]
fn a_single_character_survives() {
    assert_eq!(tokenize("验", false), ["验"]);
    assert_eq!(tokenize("缓", false), ["缓"]);
}

#[test]
fn latin_comes_first_then_the_cjk_runs_in_text_order() {
    let tokens = tokenize("cache 缓存 pool 连接池", false);
    assert_eq!(tokens, ["cache", "pool", "缓存", "连接", "接池"]);
}

#[test]
fn camel_case_is_not_split() {
    assert_eq!(tokenize("syncBlocks", false), ["syncblocks"]);
}

#[test]
fn punctuation_and_bare_digits_yield_nothing() {
    assert!(tokenize("", false).is_empty());
    assert!(tokenize("!! ... ", false).is_empty());
    assert!(tokenize("4096", false).is_empty());
    assert_eq!(tokenize("value=4096", false), ["value"]);
    assert_eq!(
        tokenize("a__b c_ snake_case_", false),
        ["a__b", "snake_case"]
    );
}

#[test]
fn digits_inside_a_cjk_run_end_it() {
    let tokens = tokenize("检测到3个异常", false);
    assert!(tokens.contains(&"检测".to_owned()), "{tokens:?}");
    assert!(tokens.contains(&"异常".to_owned()), "{tokens:?}");
    assert!(!tokens.contains(&"3".to_owned()));
}

#[test]
fn stemming_leaves_cjk_untouched() {
    assert_eq!(tokenize("身份验证", true), tokenize("身份验证", false));
}

#[test]
fn the_authenticate_family_does_not_converge() {
    assert_eq!(stem_word("authenticate"), "authenticate");
    assert_eq!(stem_word("authenticated"), "authenticat");
    assert_eq!(stem_word("authentication"), "authenticat");
}

#[test]
fn the_stemmer_strips_common_suffixes() {
    for (word, stem) in [
        ("tokens", "token"),
        ("running", "runn"),
        ("compressed", "compress"),
        ("handling", "handl"),
        ("subagents", "subagent"),
        ("queries", "query"),
        ("boxes", "box"),
        ("matches", "match"),
        ("class", "class"),
        ("payment", "pay"),
        ("darkness", "dark"),
        ("quickly", "quick"),
        ("its", "its"),
    ] {
        assert_eq!(stem_word(word), stem, "{word}");
    }
}

#[test]
fn bigrams_skip_whitespace_and_short_input() {
    assert_eq!(char_bigrams("登录"), ["登录"]);
    assert_eq!(char_bigrams("可视 化"), ["可视"]);
    assert!(char_bigrams("a").is_empty());
}

#[test]
fn term_frequencies_count_and_stem() {
    let frequencies = term_frequencies("token token 身份", true);
    assert_eq!(frequencies["token"], 2);
    assert_eq!(frequencies["身份"], 1);
}
