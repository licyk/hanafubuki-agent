// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/search/doc-cache.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the cache is a value the caller owns (one
// per session) instead of process-wide state, and its cap is in UTF-16 code
// units.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::tokenizer::{char_bigrams, term_frequencies};
use crate::text::utf16_len;

/// The default cap: 8 Mi UTF-16 code units of source text.
pub const DEFAULT_DOC_CACHE_CAP: usize = 8 * 1024 * 1024;

/// What scoring derives from a document's text.
#[derive(Clone, Debug, PartialEq)]
pub struct DocFeatures {
    /// Stemmed term frequencies (BM25).
    pub tf: HashMap<String, u32>,
    /// The number of terms (BM25's length normalization).
    pub len: u32,
    /// The lowercased text (substring and fuzzy).
    pub lower: String,
    /// The character bigrams of `lower` (fuzzy).
    pub grams: HashSet<String>,
}

impl DocFeatures {
    fn build(text: &str) -> Self {
        let tf = term_frequencies(text, true);
        let len = tf.values().sum();
        let lower = text.to_lowercase();
        let grams = char_bigrams(&lower).into_iter().collect();
        Self {
            tf,
            len,
            lower,
            grams,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocCacheInfo {
    pub entries: usize,
    /// UTF-16 code units of the cached texts.
    pub chars: usize,
}

/// Document features kept across searches, least recently used evicted
/// first. The corpus (summaries and folded text) does not change, so a
/// search tokenizes only what is new. A corpus over the cap that every
/// search scans whole is retokenized past the cap each time: raise the cap
/// rather than lower it there.
#[derive(Debug)]
pub struct DocCache {
    cap: usize,
    used: usize,
    tick: u64,
    entries: HashMap<Arc<str>, Entry>,
    by_use: BTreeMap<u64, Arc<str>>,
}

#[derive(Debug)]
struct Entry {
    features: Arc<DocFeatures>,
    tick: u64,
}

impl Default for DocCache {
    fn default() -> Self {
        Self::new(DEFAULT_DOC_CACHE_CAP)
    }
}

impl DocCache {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            used: 0,
            tick: 0,
            entries: HashMap::new(),
            by_use: BTreeMap::new(),
        }
    }

    /// The features of `text`, from the cache when it holds them. A text
    /// longer than the cap is never cached.
    pub fn features(&mut self, text: &str) -> Arc<DocFeatures> {
        self.tick += 1;
        if let Some(entry) = self.entries.get_mut(text) {
            let key = self
                .by_use
                .remove(&entry.tick)
                .unwrap_or_else(|| Arc::from(text));
            entry.tick = self.tick;
            self.by_use.insert(self.tick, key);
            return Arc::clone(&entry.features);
        }
        let features = Arc::new(DocFeatures::build(text));
        let units = utf16_len(text);
        if units > 0 && units <= self.cap {
            self.evict_down_to(self.cap - units);
            let key: Arc<str> = Arc::from(text);
            self.by_use.insert(self.tick, Arc::clone(&key));
            self.entries.insert(
                key,
                Entry {
                    features: Arc::clone(&features),
                    tick: self.tick,
                },
            );
            self.used += units;
        }
        features
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.by_use.clear();
        self.used = 0;
    }

    /// Sets the cap, evicting at once what no longer fits.
    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap.max(1);
        self.evict_down_to(self.cap);
    }

    pub fn info(&self) -> DocCacheInfo {
        DocCacheInfo {
            entries: self.entries.len(),
            chars: self.used,
        }
    }

    fn evict_down_to(&mut self, limit: usize) {
        while self.used > limit {
            let Some((_, key)) = self.by_use.pop_first() else {
                break;
            };
            self.used -= utf16_len(&key);
            self.entries.remove(&key);
        }
    }
}
