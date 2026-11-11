//! What the person configured for a session's model, which beats every
//! other source of what is known of it.

use hanafubuki_agent_model_api::{ModelReasoningEffort, ModelReasoningSupport};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

/// The facts the person configured for a model: for the standalone Agent,
/// the Provider's `models."<id>"` override, its window else the shared
/// policy's. Each fact set beats a learned, reported, catalog or built-in
/// one; an unset fact leaves them to say. A fact that does not decode is
/// unset.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfiguredModelFacts {
    #[serde(
        default,
        deserialize_with = "tolerant",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_window_tokens: Option<u64>,
    #[serde(
        default,
        deserialize_with = "tolerant",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_output_tokens: Option<u64>,
    /// Whether the model reads images. A session's input policy that
    /// disables images still wins over a configured "yes".
    #[serde(
        default,
        deserialize_with = "tolerant",
        skip_serializing_if = "Option::is_none"
    )]
    pub image_input: Option<bool>,
    /// The reasoning levels the model takes, in place of the known ones;
    /// empty: it takes no reasoning parameter.
    #[serde(
        default,
        deserialize_with = "tolerant",
        skip_serializing_if = "Option::is_none"
    )]
    pub reasoning_efforts: Option<Vec<ModelReasoningEffort>>,
}

impl AgentConfiguredModelFacts {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The configured reasoning fact: none configured, none; no levels,
    /// unsupported; else supported at those levels.
    pub(crate) fn reasoning(&self) -> Option<ModelReasoningSupport> {
        self.reasoning_efforts.as_ref().map(|efforts| {
            if efforts.is_empty() {
                ModelReasoningSupport::Unsupported
            } else {
                ModelReasoningSupport::Supported {
                    efforts: efforts.clone(),
                    default: None,
                }
            }
        })
    }
}

/// The facts a session's record holds; ones that do not decode are unset.
pub(crate) fn tolerant_configured_facts<'de, D>(
    deserializer: D,
) -> Result<AgentConfiguredModelFacts, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn tolerant<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}
