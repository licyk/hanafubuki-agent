//! What the hierarchical compression keeps beside its persisted state: the
//! content store's originals, and the interrupt the nudge still has to book.
//!
//! The content store is the history store's file of its own, not part of
//! the records: the kernel writes it whole after a commit that grew it,
//! with the record's lock released, and reads it when it loads the session.
//! The Agent keeps the full history, so the store is a cache of it: a
//! placeholder is built from the original alone (its ref, kind, size, head
//! and command), so a store that lost an entry stores it again, byte for
//! byte, at the next request that shows the message. Only the originals of
//! messages a fold covered and the window no longer holds are lost with it,
//! and `acp_retrieve` then answers that the ref has no stored original.

use hanafubuki_agent_compression::State;

use super::*;

#[derive(Clone, Default)]
pub(in crate::service) struct CompressionRuntime {
    /// The originals the content store replaced or a fold covered; none
    /// until one is stored or loaded.
    content_store: Option<Arc<AgentContentStore>>,
    /// How many entries the history store's copy holds. Entries are only
    /// ever added (a ref's first one is kept), so more here means unwritten.
    persisted_entries: usize,
    /// The token count the latest request was projected with.
    sent_tokens: Option<u64>,
    /// A user interrupted a turn sent with this count: the nudge books it
    /// once the count moves past it
    /// (`NudgeState::absorb_aborted_turn_growth`). Never persisted.
    pub(super) abort_anchor: Option<u64>,
    /// The clock a test fixed for the session's message timestamps.
    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::service) fixed_clock: Option<hanafubuki_agent_compression::LocalTime>,
}

impl CompressionRuntime {
    /// A loaded session's: the history store's copy of its content store,
    /// each entry checked against the session's state.
    pub(in crate::service) fn loaded(
        content_store: Option<AgentContentStore>,
        state: Option<&State>,
    ) -> Self {
        let content_store = content_store.and_then(|store| bound_entries(store, state?));
        Self {
            persisted_entries: content_store.as_ref().map_or(0, |store| store.by_ref.len()),
            content_store: content_store.map(Arc::new),
            ..Self::default()
        }
    }

    /// A fork's or a child's, over the state it starts with: the source's
    /// originals that state still binds, written as the new session's own
    /// at its first request.
    pub(in crate::service) fn forked(&self, state: Option<&State>) -> Self {
        let content_store = self
            .content_store
            .as_deref()
            .and_then(|store| bound_entries(store.clone(), state?));
        Self {
            content_store: content_store.map(Arc::new),
            ..Self::default()
        }
    }

    pub(in crate::service) fn content_store(&self) -> Option<&AgentContentStore> {
        self.content_store.as_deref()
    }

    pub(in crate::service) fn keep_content_store(&mut self, store: AgentContentStore) {
        self.content_store = Some(Arc::new(store));
    }

    /// The store, when it holds entries its copy does not.
    pub(in crate::service) fn unpersisted(&self) -> Option<Arc<AgentContentStore>> {
        self.content_store
            .clone()
            .filter(|store| store.by_ref.len() > self.persisted_entries)
    }

    pub(in crate::service) fn mark_persisted(&mut self, entries: usize) {
        self.persisted_entries = self.persisted_entries.max(entries);
    }

    /// A request was projected with `tokens`; an interrupt it booked is
    /// done.
    pub(in crate::service) fn sent(&mut self, tokens: u64, abort_booked: bool) {
        self.sent_tokens = Some(tokens);
        if abort_booked {
            self.abort_anchor = None;
        }
    }

    /// A user interrupt ended the turn: its growth is booked by a later
    /// request.
    pub(in crate::service) fn interrupted(&mut self) {
        if let Some(tokens) = self.sent_tokens {
            self.abort_anchor = Some(tokens);
        }
    }
}

/// `store` without the entries `state` does not bind to the same message:
/// a state rebuilt from the history may number its refs differently, and a
/// ref must never show another message's placeholder. None when nothing is
/// left.
fn bound_entries(mut store: AgentContentStore, state: &State) -> Option<AgentContentStore> {
    store
        .by_ref
        .retain(|r#ref, entry| state.message_refs.by_raw.get(&entry.raw_id) == Some(r#ref));
    let used: std::collections::HashSet<&str> = store
        .by_ref
        .values()
        .map(|entry| entry.hash.as_str())
        .collect();
    store.by_hash.retain(|hash, _| used.contains(hash.as_str()));
    (!store.by_ref.is_empty()).then_some(store)
}

impl AgentService {
    /// Writes the session's content store when it holds entries the history
    /// store's copy does not, with the record's lock released. A failed
    /// write is logged and tried again at the next change: the store is a
    /// cache of the history.
    pub(in crate::service) fn persist_content_store(&self, session_id: AgentSessionId) {
        let Ok(cell) = self.session(session_id) else {
            return;
        };
        let Some(store) = cell
            .lock()
            .ok()
            .and_then(|record| record.runtime.compression_runtime.unpersisted())
        else {
            return;
        };
        match self
            .inner
            .history_store
            .write_compression_store(session_id, &store)
        {
            Ok(()) => {
                if let Ok(mut record) = cell.lock() {
                    record
                        .runtime
                        .compression_runtime
                        .mark_persisted(store.by_ref.len());
                }
            }
            Err(error) => log::debug!(
                "Agent session {session_id} content store not written: {}",
                bounded_diagnostic(error.to_string())
            ),
        }
    }

    /// The content store the history store holds for a loaded session; none
    /// when it holds none or it could not be read.
    pub(in crate::service) fn load_content_store(
        &self,
        session_id: AgentSessionId,
    ) -> Option<AgentContentStore> {
        self.inner
            .history_store
            .load_compression_store(session_id)
            .unwrap_or_else(|error| {
                log::warn!(
                    "Agent session {session_id} content store not read: {}",
                    bounded_diagnostic(error.to_string())
                );
                None
            })
    }
}
