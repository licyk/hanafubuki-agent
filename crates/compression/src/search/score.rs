// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/search/algorithms/{bm25,fuzzy,hybrid,substring}.ts` of
// acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026 ranxianglei.
// MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the algorithms are an enum, not a registry
// (no embedding-based semantic search), and a score is returned per
// document, in the documents' order.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::cache::{DocCache, DocFeatures};
use super::tokenizer::{char_bigrams, is_cjk, is_space, tokenize};
use super::SearchDoc;
use crate::text::utf16_len;

const K1: f64 = 1.2;
const B: f64 = 0.75;
/// The hybrid's weights: BM25 brings precision on real terms, the fuzzy
/// bigrams recall on typos and partial words.
const W_BM25: f64 = 0.7;
const W_FUZZY: f64 = 0.3;

/// How documents are scored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Algorithm {
    /// BM25 and fuzzy, each normalized to its best score, weighted 0.7 and
    /// 0.3.
    #[default]
    Hybrid,
    /// BM25 (k1 1.2, b 0.75) over stemmed terms.
    Bm25,
    /// The share of the query's character bigrams a document holds.
    Fuzzy,
    /// Occurrences of each query term, lowercased.
    Substring,
}

pub(super) fn score(
    algorithm: Algorithm,
    docs: &[SearchDoc],
    query: &str,
    cache: &mut DocCache,
) -> Vec<f64> {
    let features: Vec<Arc<DocFeatures>> =
        docs.iter().map(|doc| cache.features(&doc.text)).collect();
    match algorithm {
        Algorithm::Hybrid => hybrid(&features, query),
        Algorithm::Bm25 => bm25(&features, query),
        Algorithm::Fuzzy => fuzzy(&features, query),
        Algorithm::Substring => substring(&features, query),
    }
}

fn bm25(features: &[Arc<DocFeatures>], query: &str) -> Vec<f64> {
    let terms = tokenize(query, true);
    if terms.is_empty() {
        return vec![0.0; features.len()];
    }
    let n = features.len() as f64;
    let total_len: f64 = features.iter().map(|doc| f64::from(doc.len)).sum();
    let average = total_len / n.max(1.0);
    let mut idf: HashMap<&str, f64> = HashMap::new();
    for term in terms.iter().collect::<HashSet<_>>() {
        let df = features
            .iter()
            .filter(|doc| doc.tf.contains_key(term))
            .count() as f64;
        idf.insert(term, (1.0 + (n - df + 0.5) / (df + 0.5)).ln());
    }
    let average = if average == 0.0 { 1.0 } else { average };
    features
        .iter()
        .map(|doc| {
            let mut score = 0.0;
            for term in &terms {
                let frequency = f64::from(doc.tf.get(term).copied().unwrap_or(0));
                if frequency == 0.0 {
                    continue;
                }
                score += (idf[term.as_str()] * (frequency * (K1 + 1.0)))
                    / (frequency + K1 * (1.0 - B + (B * f64::from(doc.len)) / average));
            }
            score
        })
        .collect()
}

/// Query tokens of four or more units, or CJK words of two or more: short
/// Latin tokens overlap every document, while most CJK words are two
/// characters.
fn fuzzy(features: &[Arc<DocFeatures>], query: &str) -> Vec<f64> {
    let lower = query.to_lowercase();
    let grams: HashSet<String> = lower
        .split(|character: char| is_space(character) || character == ',')
        .filter(|token| {
            let units = utf16_len(token);
            units >= 4 || (units >= 2 && token.chars().any(is_cjk))
        })
        .flat_map(char_bigrams)
        .collect();
    if grams.is_empty() {
        return vec![0.0; features.len()];
    }
    features
        .iter()
        .map(|doc| {
            let hits = grams
                .iter()
                .filter(|gram| doc.grams.contains(*gram))
                .count();
            hits as f64 / grams.len() as f64
        })
        .collect()
}

fn hybrid(features: &[Arc<DocFeatures>], query: &str) -> Vec<f64> {
    let bm = bm25(features, query);
    let fz = fuzzy(features, query);
    let max_bm = bm.iter().copied().fold(1e-9, f64::max);
    let max_fz = fz.iter().copied().fold(1e-9, f64::max);
    bm.iter()
        .zip(&fz)
        .map(|(bm, fz)| W_BM25 * (bm / max_bm) + W_FUZZY * (fz / max_fz))
        .collect()
}

fn substring(features: &[Arc<DocFeatures>], query: &str) -> Vec<f64> {
    let lower = query.to_lowercase();
    let terms: Vec<&str> = lower
        .split(is_space)
        .filter(|term| !term.is_empty())
        .collect();
    features
        .iter()
        .map(|doc| {
            terms
                .iter()
                .map(|term| doc.lower.matches(term).count() as f64)
                .sum()
        })
        .collect()
}
