//! The latest projection, kept for the next call with the same inputs. A
//! sampling step builds its request twice, for the admission estimate and
//! for the request it sends, from the same history, state and settings, so
//! the second takes the projection the first made instead of projecting
//! again.

use hanafubuki_agent_compression::{Config, TokenCountSource};

use super::*;
use crate::{AgentCompressionState, AgentContentStore};

/// Everything a projection is computed from. The history items, the state
/// and the content store are compared by identity: the key holds them, so
/// no address is reused while it is kept, and the kernel changes a shared
/// item or state only by replacing it (`Arc::make_mut` copies). The clock's
/// reading is left out: a reused projection keeps the stamps of the call
/// that made it, earlier in the same step.
pub(super) struct ProjectionKey {
    pub(super) items: Vec<Arc<AgentHistoryItem>>,
    pub(super) state: Option<Arc<AgentCompressionState>>,
    pub(super) content_store: Option<Arc<AgentContentStore>>,
    pub(super) mode: CompressionMode,
    pub(super) config: Config,
    /// The clock's offset while message timestamps are on.
    pub(super) clock_offset: Option<i32>,
    pub(super) token_count: (u64, TokenCountSource),
    pub(super) abort_anchor: Option<u64>,
    pub(super) anthropic: bool,
}

impl PartialEq for ProjectionKey {
    fn eq(&self, other: &Self) -> bool {
        self.items.len() == other.items.len()
            && self
                .items
                .iter()
                .zip(&other.items)
                .all(|(item, other)| Arc::ptr_eq(item, other))
            && same_arc(&self.state, &other.state)
            && same_arc(&self.content_store, &other.content_store)
            && self.mode == other.mode
            && self.config == other.config
            && self.clock_offset == other.clock_offset
            && self.token_count == other.token_count
            && self.abort_anchor == other.abort_anchor
            && self.anthropic == other.anthropic
    }
}

fn same_arc<T>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

/// Never persisted; shared by the record's rollback copies, which is sound
/// since a projection is reused only for the inputs it was made from.
#[derive(Clone, Default)]
pub(in crate::service) struct ProjectionCache(
    Arc<parking_lot::Mutex<Option<(ProjectionKey, RequestProjection)>>>,
);

impl ProjectionCache {
    /// The projection kept for `key`, taken out: the step's second call is
    /// the last that can use it.
    pub(super) fn take(&self, key: &ProjectionKey) -> Option<RequestProjection> {
        let mut kept = self.0.lock();
        if kept.as_ref().is_some_and(|(kept, _)| kept == key) {
            return kept.take().map(|(_, projection)| projection);
        }
        None
    }

    pub(super) fn keep(&self, key: ProjectionKey, projection: RequestProjection) {
        *self.0.lock() = Some((key, projection));
    }
}
