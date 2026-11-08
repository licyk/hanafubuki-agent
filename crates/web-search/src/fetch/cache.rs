use super::{
    CachedDocument, FetchedDocument, WebPageFetcher, MAX_DOCUMENT_CACHE_BYTES,
    MAX_DOCUMENT_CACHE_ENTRIES,
};
use std::sync::Arc;
use std::time::Instant;

impl WebPageFetcher {
    pub(super) fn cached_document(
        &self,
        requested_url: &str,
        generation: &str,
    ) -> Option<Arc<FetchedDocument>> {
        if self.document_cache_ttl.is_zero() {
            return None;
        }
        let mut cache = self.document_cache.lock();
        cache.retain(|entry| entry.inserted_at.elapsed() <= self.document_cache_ttl);
        cache
            .iter()
            .find(|entry| {
                entry.requested_url == requested_url && entry.document.generation == generation
            })
            .map(|entry| entry.document.clone())
    }

    pub(super) fn store_document(&self, requested_url: String, document: Arc<FetchedDocument>) {
        if self.document_cache_ttl.is_zero() {
            return;
        }
        let mut cache = self.document_cache.lock();
        cache.retain(|entry| {
            entry.inserted_at.elapsed() <= self.document_cache_ttl
                && !(entry.requested_url == requested_url
                    && entry.document.generation == document.generation)
        });
        let mut retained_bytes = cache
            .iter()
            .map(|entry| entry.document.content.len())
            .sum::<usize>();
        while cache.len() >= MAX_DOCUMENT_CACHE_ENTRIES
            || retained_bytes.saturating_add(document.content.len()) > MAX_DOCUMENT_CACHE_BYTES
        {
            let Some(removed) = cache.pop_front() else {
                break;
            };
            retained_bytes = retained_bytes.saturating_sub(removed.document.content.len());
        }
        cache.push_back(CachedDocument {
            requested_url,
            inserted_at: Instant::now(),
            document,
        });
    }
}
