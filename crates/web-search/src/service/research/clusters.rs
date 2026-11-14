// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use crate::WebSearchResult;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub source: WebSearchResult,
    pub key: String,
    pub cluster: Option<String>,
    pub representative: bool,
    pub round: u8,
}
pub(super) fn key(address: &str) -> String {
    let Ok(url) = url::Url::parse(address) else {
        return address.into();
    };
    let host = url.host_str().unwrap_or_default();
    let host = ["www.", "m.", "mobile."]
        .into_iter()
        .find_map(|p| host.strip_prefix(p))
        .unwrap_or(host);
    format!(
        "{}{}{}",
        host.to_lowercase(),
        url.path().trim_end_matches('/'),
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    )
}
pub(super) fn build(sources: &[WebSearchResult]) -> Vec<Candidate> {
    let mut seen = BTreeSet::new();
    let mut titles = BTreeMap::new();
    let mut candidates = Vec::new();
    for source in sources {
        let key = key(&source.url);
        if !seen.insert(key.clone()) {
            continue;
        }
        let title: String = source
            .title
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let representative = !titles.contains_key(&title);
        let next = titles.len() + 1;
        let cluster = (!title.is_empty()).then(|| {
            titles
                .entry(title)
                .or_insert_with(|| format!("c{next}"))
                .clone()
        });
        candidates.push(Candidate {
            source: source.clone(),
            key,
            cluster,
            representative,
            round: 1,
        });
    }
    candidates
}
pub(super) fn cosine(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let denominator =
        a.iter().map(|v| v * v).sum::<f64>().sqrt() * b.iter().map(|v| v * v).sum::<f64>().sqrt();
    if denominator > 0.0 {
        dot / denominator
    } else {
        0.0
    }
}
pub(super) fn semantic_tags(candidates: &mut [Candidate], vectors: &[Vec<f64>]) {
    let mut parents: Vec<usize> = (0..candidates.len()).collect();
    for i in 0..candidates.len() {
        for j in i + 1..candidates.len() {
            if candidates[i].cluster.is_some() || candidates[j].cluster.is_some() {
                continue;
            }
            if vectors
                .get(i)
                .zip(vectors.get(j))
                .is_some_and(|(a, b)| cosine(a, b) > 0.92)
            {
                let root_i = root(&parents, i);
                let root_j = root(&parents, j);
                parents[root_i] = root_j;
            }
        }
    }
    let mut sizes = BTreeMap::new();
    for (i, candidate) in candidates.iter().enumerate() {
        if candidate.cluster.is_none() {
            *sizes.entry(root(&parents, i)).or_insert(0) += 1;
        }
    }
    for (i, candidate) in candidates.iter_mut().enumerate() {
        let root = root(&parents, i);
        if candidate.cluster.is_none() && sizes.get(&root).copied().unwrap_or(0) > 1 {
            candidate.cluster = Some(format!("s{root}"));
        }
    }
}
fn root(parents: &[usize], mut index: usize) -> usize {
    while parents[index] != index {
        index = parents[index];
    }
    index
}
pub(super) fn numerically_consistent(a: &str, b: &str) -> bool {
    let Ok(pattern) = regex::Regex::new(r"\d+(?:\.\d+)?") else {
        return false;
    };
    let left: BTreeSet<_> = pattern.find_iter(a).map(|m| m.as_str()).collect();
    let right: BTreeSet<_> = pattern.find_iter(b).map(|m| m.as_str()).collect();
    left.is_empty() || right.is_empty() || left == right
}
