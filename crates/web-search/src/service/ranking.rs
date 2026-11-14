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

// Derived from dsh-free-search 6ab5168 (MIT), dedupe.js and multi_policy.js.

use super::normalize::normalize_result_url;
use crate::WebSearchResult;
use std::collections::{HashMap, VecDeque};

pub(super) fn merge_rank(results: Vec<WebSearchResult>, freshness: bool) -> Vec<WebSearchResult> {
    let mut merged: Vec<WebSearchResult> = Vec::new();
    let mut urls = HashMap::new();
    for mut item in results {
        let Some(url) = normalize_result_url(&item.url) else {
            continue;
        };
        let key = url.trim_end_matches('/').to_string();
        if let Some(index) = urls.get(&key).copied() {
            let prior: &mut WebSearchResult = &mut merged[index];
            for engine in item.engines {
                if !prior.engines.contains(&engine) {
                    prior.engines.push(engine);
                }
            }
            if prior.snippet.len() < item.snippet.len() {
                prior.snippet = item.snippet;
            }
        } else {
            item.url = url;
            urls.insert(key, merged.len());
            merged.push(item);
        }
    }
    let mut signatures = Vec::new();
    merged.retain(|item| {
        let signature = min_hash(&format!("{} {}", item.title, item.snippet));
        if signatures.iter().any(|prior: &[u32; 16]| {
            prior
                .iter()
                .zip(signature)
                .filter(|(a, b)| **a == *b)
                .count()
                >= 13
        }) {
            return false;
        }
        signatures.push(signature);
        true
    });
    if freshness {
        merged.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    }
    let mut buckets: Vec<(String, VecDeque<WebSearchResult>)> = Vec::new();
    for item in merged {
        let domain = domain(&item.url);
        if let Some((_, group)) = buckets.iter_mut().find(|(key, _)| *key == domain) {
            group.push_back(item);
        } else {
            buckets.push((domain, VecDeque::from([item])));
        }
    }
    let mut out = Vec::new();
    loop {
        let before = out.len();
        for (_, group) in &mut buckets {
            if let Some(item) = group.pop_front() {
                out.push(item);
            }
        }
        if before == out.len() {
            break;
        }
    }
    out
}

pub(crate) fn domain(address: &str) -> String {
    let Ok(url) = reqwest::Url::parse(address) else {
        return String::new();
    };
    let host = url.host_str().unwrap_or("");
    let parts = host.split('.').collect::<Vec<_>>();
    if parts.len() < 3 || host.parse::<std::net::IpAddr>().is_ok() {
        return host.into();
    }
    let tail = parts[parts.len() - 2..].join(".");
    let multi = [
        "com.cn", "net.cn", "org.cn", "gov.cn", "edu.cn", "ac.cn", "com.hk", "org.hk", "net.hk",
        "edu.hk", "gov.hk", "com.tw", "org.tw", "net.tw", "edu.tw", "gov.tw", "co.jp", "or.jp",
        "ne.jp", "ac.jp", "go.jp", "co.kr", "or.kr", "ne.kr", "go.kr", "re.kr", "co.uk", "org.uk",
        "ac.uk", "gov.uk", "me.uk", "net.uk", "sch.uk", "com.au", "net.au", "org.au", "edu.au",
        "gov.au", "co.nz", "net.nz", "org.nz", "govt.nz", "ac.nz", "co.za", "org.za", "net.za",
        "gov.za", "co.in", "net.in", "org.in", "gov.in", "ac.in", "com.sg", "com.my", "com.ph",
        "com.pk", "com.vn", "co.th", "co.id", "com.br", "com.mx", "com.ar", "com.tr", "com.ua",
        "com.pl", "com.ru", "co.il", "com.sa", "com.eg", "com.ng", "co.ke",
    ];
    if multi.contains(&tail.as_str()) {
        parts[parts.len() - 3..].join(".")
    } else {
        tail
    }
}

fn min_hash(text: &str) -> [u32; 16] {
    let text = text
        .split_whitespace()
        .filter(|word| !word.starts_with("http://") && !word.starts_with("https://"))
        .collect::<String>()
        .to_lowercase();
    let normalized = text
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();
    let units = normalized.encode_utf16().collect::<Vec<_>>();
    if units.is_empty() {
        return [0; 16];
    }
    let size = if units.len() >= 6 { 3 } else { units.len() };
    std::array::from_fn(|index| {
        let seed = (index as u32 + 1).wrapping_mul(0x9e3779b1);
        units
            .windows(size)
            .map(|part| {
                part.iter().fold(2166136261 ^ seed, |h, unit| {
                    (h ^ u32::from(*unit)).wrapping_mul(16777619)
                })
            })
            .min()
            .unwrap_or(0)
    })
}
