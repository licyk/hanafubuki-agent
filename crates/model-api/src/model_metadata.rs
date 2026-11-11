//! What is known about a model route, fact by fact, each with its source.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::ModelReasoningEffort;

/// What is known about a route; every fact may be unknown. A fact that does
/// not decode is unknown, so one bad fact never loses the others.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadata {
    #[serde(default, deserialize_with = "tolerant")]
    pub context_window: Option<Sourced<u64>>,
    #[serde(default, deserialize_with = "tolerant")]
    pub max_output_tokens: Option<Sourced<u64>>,
    #[serde(default, deserialize_with = "tolerant")]
    pub reasoning: Option<Sourced<ModelReasoningSupport>>,
    #[serde(default, deserialize_with = "tolerant")]
    pub image_input: Option<Sourced<bool>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelReasoningSupport {
    /// It reasons, at these efforts (empty: levels not reported), with this
    /// default. An effort that does not parse is left out.
    Supported {
        #[serde(default, deserialize_with = "tolerant_efforts")]
        efforts: Vec<ModelReasoningEffort>,
        #[serde(default, deserialize_with = "tolerant")]
        default: Option<ModelReasoningEffort>,
    },
    /// It takes no reasoning parameter.
    Unsupported,
}

/// A fact and where it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sourced<T> {
    pub value: T,
    pub source: ModelMetadataSource,
    /// Milliseconds since the Unix epoch.
    #[serde(default)]
    pub observed_at_ms: u64,
}

/// Where a fact came from, in precedence order: the person's configuration,
/// a Provider's refusal, its listing, a third-party catalog, the built-in
/// table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelMetadataSource {
    Configured,
    Learned,
    Reported,
    Catalog,
    BuiltIn,
}

fn tolerant<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

fn tolerant_efforts<'de, D>(deserializer: D) -> Result<Vec<ModelReasoningEffort>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|effort| ModelReasoningEffort::parse(effort.as_str()?).ok())
        .collect())
}
