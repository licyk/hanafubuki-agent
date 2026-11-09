// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `doc-cache.test.ts` against a caller-owned cache.

use std::sync::Arc;

use hanafubuki_agent_compression::{
    search_blocks, DocCache, Role, SearchDoc, SearchDocKind, SearchOptions,
};

#[test]
fn the_same_text_returns_the_cached_features() {
    let mut cache = DocCache::default();
    let a = cache.features("缓存命中 cache hit 身份验证");
    let b = cache.features("缓存命中 cache hit 身份验证");
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(cache.info().entries, 1);
}

#[test]
fn different_texts_have_independent_features() {
    let mut cache = DocCache::default();
    let a = cache.features("身份验证失败");
    let b = cache.features("请求超时处理");
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(a.tf.contains_key("身份") && !a.tf.contains_key("请求"));
    assert!(b.tf.contains_key("请求") && !b.tf.contains_key("身份"));
}

#[test]
fn the_derived_fields_are_correct() {
    let mut cache = DocCache::default();
    let features = cache.features("The quick Cache cache CACHE, 缓存 缓存 hit");
    assert!(features.lower.starts_with("the quick cache"));
    assert_eq!(features.tf["cache"], 3);
    assert_eq!(features.tf["缓存"], 2);
    assert_eq!(features.len, features.tf.values().sum::<u32>());
    assert!(features.grams.contains("qu"));
    assert!(features.grams.contains("缓存"));
}

#[test]
fn clearing_forces_a_rebuild() {
    let mut cache = DocCache::default();
    let a = cache.features("缓存命中 test-clear");
    cache.clear();
    let b = cache.features("缓存命中 test-clear");
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(cache.info().entries, 1);
}

#[test]
fn a_small_cap_evicts_the_least_recently_used() {
    let mut cache = DocCache::new(20);
    cache.features(&"a".repeat(10));
    cache.features(&"b".repeat(10));
    assert_eq!((cache.info().entries, cache.info().chars), (2, 20));
    cache.features(&"c".repeat(15));
    assert_eq!((cache.info().entries, cache.info().chars), (1, 15));
}

#[test]
fn a_text_used_again_stays_cached() {
    let mut cache = DocCache::new(25);
    let a = cache.features(&"a".repeat(10));
    cache.features(&"b".repeat(10));
    cache.features(&"a".repeat(10));
    cache.features(&"c".repeat(15));
    assert!(Arc::ptr_eq(&a, &cache.features(&"a".repeat(10))));
}

#[test]
fn an_unbounded_cap_keeps_the_whole_corpus() {
    let mut cache = DocCache::new(usize::MAX);
    for text in ["a", "b", "c"] {
        cache.features(&text.repeat(100));
    }
    assert_eq!((cache.info().entries, cache.info().chars), (3, 300));
}

#[test]
fn lowering_the_cap_evicts_at_once() {
    let mut cache = DocCache::default();
    cache.features(&"x".repeat(100));
    assert_eq!(cache.info().entries, 1);
    cache.set_cap(50);
    assert_eq!((cache.info().entries, cache.info().chars), (0, 0));
}

#[test]
fn a_text_over_the_cap_is_never_cached() {
    let mut cache = DocCache::new(100);
    let features = cache.features(&"alpha ".repeat(100));
    assert!(features.tf.contains_key("alpha"));
    assert_eq!(cache.info().entries, 0);
}

#[test]
fn a_warm_cache_gives_the_same_scores() {
    let doc = |kind, r#ref: &str, text: &str, role| SearchDoc {
        kind,
        r#ref: r#ref.to_owned(),
        text: text.to_owned(),
        title: r#ref.to_owned(),
        role,
        block_id: Some("b1".to_owned()),
        tier: Some(1),
        tokens: None,
    };
    let docs = [
        doc(
            SearchDocKind::Block,
            "b1",
            "缓存服务 命中率 0.87 身份验证失败 请求超时",
            None,
        ),
        doc(
            SearchDocKind::Message,
            "m1",
            "the auth flow failed twice, then the cache warm-up finished",
            Some(Role::Tool),
        ),
        doc(
            SearchDocKind::Block,
            "b2",
            "auth failure 身份验证 cache warmup 缓存预热",
            None,
        ),
    ];
    let mut cache = DocCache::default();
    let options = SearchOptions::default();
    let scored = |cache: &mut DocCache| -> Vec<(String, f64)> {
        search_blocks(&docs, "缓存 身份验证", &options, cache)
            .into_iter()
            .map(|result| (result.r#ref, result.score))
            .collect()
    };
    let cold = scored(&mut cache);
    let warm = scored(&mut cache);
    assert!(!cold.is_empty());
    assert_eq!(warm, cold);
}
