//! What is known about a model route, fact by fact, each with its source.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{ModelReasoningEffort, ModelStructuredOutputCapability};

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
    /// The best rung of a structured request the route still takes; only
    /// a refusal teaches it (`Learned`), and it only lowers the protocol's.
    #[serde(default, deserialize_with = "tolerant")]
    pub structured_output: Option<Sourced<ModelStructuredOutputCapability>>,
}

impl ModelMetadata {
    /// The reasoning efforts the model may be given, as the person or the
    /// Provider named them. Only a configured or reported fact names
    /// levels, and only a configured, reported or learned one says the
    /// model takes none: models.dev's catalogue and the built-in table are
    /// guesses, so what they say is unknown here.
    pub fn known_reasoning(&self) -> KnownReasoning {
        let Some(fact) = &self.reasoning else {
            return KnownReasoning::Unknown;
        };
        match (&fact.value, fact.source) {
            (
                ModelReasoningSupport::Supported { efforts, default },
                ModelMetadataSource::Configured | ModelMetadataSource::Reported,
            ) if !efforts.is_empty() => KnownReasoning::Levels {
                efforts: efforts.clone(),
                default: default.clone(),
            },
            (
                ModelReasoningSupport::Unsupported,
                ModelMetadataSource::Configured
                | ModelMetadataSource::Reported
                | ModelMetadataSource::Learned,
            ) => KnownReasoning::Unsupported {
                learned: fact.source == ModelMetadataSource::Learned,
            },
            _ => KnownReasoning::Unknown,
        }
    }
}

/// The reasoning efforts known for a model ([`ModelMetadata::known_reasoning`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnownReasoning {
    /// It takes these, with this default.
    Levels {
        efforts: Vec<ModelReasoningEffort>,
        default: Option<ModelReasoningEffort>,
    },
    /// It takes none; `learned`: a refusal of the Provider taught it.
    Unsupported {
        learned: bool,
    },
    Unknown,
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
/// a Provider's refusal, its listing, models.dev's catalogue (`Catalog`,
/// shown as `models.dev`), the built-in table.
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
