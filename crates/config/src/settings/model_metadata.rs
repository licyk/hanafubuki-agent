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

/// Strict as the settings patch that carries it (`agent_settings_write`);
/// the file is read field by field (`decode_model_metadata`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentModelMetadataSettings {
    pub discovery: AgentModelDiscovery,
    /// Whether the Agent downloads models.dev's public catalogue (once a
    /// day, from this machine) to fill what nothing else says of a model.
    /// Off by default: it is a request to a third party.
    pub models_dev: bool,
}

/// Each setting falls back on its own.
pub(crate) fn decode_model_metadata(
    fields: &mut Fields<'_>,
    settings: &mut AgentModelMetadataSettings,
) {
    settings.discovery = fields.read("discovery", settings.discovery);
    settings.models_dev = fields.read("modelsDev", settings.models_dev);
}
