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

use super::{CacheEntry, CacheKey, WebSearchService};
use crate::model::WebSearchResponse;
use std::time::Instant;

impl WebSearchService {
    pub(super) fn cached(&self, key: &CacheKey) -> Option<WebSearchResponse> {
        if !self.inner.config.search.cache_enabled {
            return None;
        }
        let mut cache = self.inner.cache.lock();
        let ttl = self.inner.config.cache_ttl;
        cache.retain(|entry| entry.inserted_at.elapsed() <= ttl);
        cache
            .iter()
            .find(|entry| &entry.key == key)
            .map(|entry| entry.response.clone())
    }

    pub(super) fn store_cache(&self, key: CacheKey, response: WebSearchResponse) {
        // Do not turn transient total-provider failures into a multi-minute
        // outage. Successful fallback responses remain cacheable even when
        // they retain a safe partial-failure diagnostic.
        if !self.inner.config.search.cache_enabled
            || self.inner.config.cache_ttl.is_zero()
            || (response.results.is_empty()
                && response.answers.is_empty()
                && !response.partial_failures.is_empty())
        {
            return;
        }
        let mut cache = self.inner.cache.lock();
        cache.retain(|entry| entry.key != key);
        while cache.len() >= self.inner.config.maximum_cache_entries {
            cache.pop_front();
        }
        cache.push_back(CacheEntry {
            key,
            inserted_at: Instant::now(),
            response,
        });
    }
}
