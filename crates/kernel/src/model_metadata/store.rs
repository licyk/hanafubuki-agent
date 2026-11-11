//! Where the registry's facts are kept between runs, and their persisted
//! form.

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use hanafubuki_agent_model_api::{ModelEndpointKey, ModelRouteKey};

use super::models_dev::{serialize_copy, tolerant_copy, ModelsDevCopy};
use super::state::{ListingState, RouteFacts};

/// Keeps the model metadata registry's facts between runs. The kernel
/// never touches the data directory: its owner injects the store.
pub trait ModelMetadataStore: Send + Sync {
    /// What the last run saved: empty when nothing was, or when what was
    /// does not read. Called once, when the kernel is built.
    fn load(&self) -> ModelMetadataSnapshot;

    /// Keeps `snapshot` in place of what was saved. Called after every
    /// change, under the registry's lock and maybe on a turn's task, so it
    /// hands the snapshot on without waiting for a disk; a failure is its
    /// own to log.
    fn save(&self, snapshot: ModelMetadataSnapshot);
}

/// Keeps nothing: the facts last as long as the kernel.
pub(crate) struct NoopModelMetadataStore;

impl ModelMetadataStore for NoopModelMetadataStore {
    fn load(&self) -> ModelMetadataSnapshot {
        ModelMetadataSnapshot::default()
    }

    fn save(&self, _snapshot: ModelMetadataSnapshot) {}
}

/// The registry's facts as a store keeps them. It is the Agent's own
/// persisted data, so it is read tolerantly: an unknown field is ignored, a
/// route or listing that does not read is left out, a fact that does not
/// read is unknown, and a copy of models.dev that does not read is none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadataSnapshot {
    #[serde(default, deserialize_with = "tolerant_entries")]
    pub(super) routes: Vec<RouteRecord>,
    #[serde(default, deserialize_with = "tolerant_entries")]
    pub(super) listings: Vec<ListingRecord>,
    /// The copy of models.dev's catalogue, when one was fetched.
    #[serde(
        default,
        deserialize_with = "tolerant_copy",
        serialize_with = "serialize_copy",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) models_dev: Option<Arc<ModelsDevCopy>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RouteRecord {
    pub(super) route: ModelRouteKey,
    #[serde(flatten)]
    pub(super) facts: RouteFacts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ListingRecord {
    pub(super) endpoint: ModelEndpointKey,
    #[serde(flatten)]
    pub(super) listing: ListingState,
}

fn tolerant_entries<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    let items = match value {
        Value::Array(items) => items,
        _ => Vec::new(),
    };
    let total = items.len();
    let entries: Vec<T> = items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect();
    if entries.len() < total {
        log::warn!(
            "model metadata: {} saved entries do not read and are left out",
            total - entries.len()
        );
    }
    Ok(entries)
}

/// A field that does not read is its default.
pub(super) fn tolerant_or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}
