//! The copy of models.dev's catalogue the registry keeps (when the person
//! turned it on, the facade downloads it): when it was fetched, how long it
//! is used, and its persisted form.

use std::sync::Arc;

use hanafubuki_agent_model_api::ModelsDevCatalog;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::state::DAY_MS;

/// How long a copy is used: a copy that cannot be refreshed (models.dev
/// unreachable) keeps answering for a week, as a learned fact does. The
/// facade refreshes it once a day.
pub(super) const MODELS_DEV_TTL_MS: u64 = 7 * DAY_MS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModelsDevCopy {
    /// Milliseconds since the Unix epoch.
    pub(super) fetched_at_ms: u64,
    pub(super) catalog: ModelsDevCatalog,
}

impl ModelsDevCopy {
    pub(super) fn live(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.fetched_at_ms) <= MODELS_DEV_TTL_MS
    }
}

/// The copy is shared, not cloned, by each snapshot.
pub(super) fn serialize_copy<S: Serializer>(
    copy: &Option<Arc<ModelsDevCopy>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    copy.as_deref().serialize(serializer)
}

/// A copy that does not read is none: it is fetched again.
pub(super) fn tolerant_copy<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Arc<ModelsDevCopy>>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<ModelsDevCopy>(value)
        .ok()
        .map(Arc::new))
}
