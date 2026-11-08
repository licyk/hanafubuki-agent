use super::{CacheEntry, CacheKey, WebSearchService};
use crate::model::WebSearchResponse;
use std::time::Instant;

impl WebSearchService {
    pub(super) fn cached(&self, key: &CacheKey) -> Option<WebSearchResponse> {
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
        if self.inner.config.cache_ttl.is_zero()
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
