//! `[modelMetadata]`: how the Agent learns what the models it reaches can
//! do.

use serde::{Deserialize, Serialize};

use crate::fields::Fields;

/// Whether the Agent lists a Provider's models by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelDiscovery {
    /// In the background, at most once a day per Provider: at start for
    /// the default Provider, and when a session's Provider or model is
    /// selected.
    #[default]
    Auto,
    /// Never by itself. The listings a person asks for (the model picker,
    /// `/model`, a Provider's test, `doctor --online`) are still recorded.
    Off,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelMetadataSettings {
    pub discovery: AgentModelDiscovery,
}

/// Each setting falls back on its own.
pub(crate) fn decode_model_metadata(
    fields: &mut Fields<'_>,
    settings: &mut AgentModelMetadataSettings,
) {
    settings.discovery = fields.read("discovery", settings.discovery);
}
