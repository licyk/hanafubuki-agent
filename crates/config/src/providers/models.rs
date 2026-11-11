//! Per-model overrides (`providers[].models."<model id>"`): what the person
//! configures for one model of a Provider, beside the shared model policy.

use std::collections::BTreeMap;

use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fields::Fields;

/// What one model is configured as. Each fact set beats what the Agent
/// learned from the Provider's refusals, what its model list reports and
/// the built-in table; for this model, the window also beats the policy's
/// `contextWindowTokens`. An unset fact leaves the others to say.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelOverrideSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "AgentModelImageInput::is_auto")]
    pub image_input: AgentModelImageInput,
    /// The levels the model takes, in place of the known ones; empty: it
    /// takes no reasoning parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<ModelReasoningEffort>>,
}

/// Whether the model reads images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelImageInput {
    /// What is known of the model: its listing, a refusal.
    #[default]
    Auto,
    /// Images are sent whatever is known; a refusal still falls back for
    /// its request. The policy's `imageInput = "disabled"` still wins.
    Supported,
    /// Images are never sent.
    Unsupported,
}

impl AgentModelImageInput {
    fn is_auto(&self) -> bool {
        *self == Self::Auto
    }

    /// The configured fact: none for `Auto`.
    pub fn configured(self) -> Option<bool> {
        match self {
            Self::Auto => None,
            Self::Supported => Some(true),
            Self::Unsupported => Some(false),
        }
    }
}

const MAX_MODEL_ID_BYTES: usize = 256;

/// The overrides of the Provider at `providers[index]`, model by model and
/// field by field: a field that cannot be used is unset, a model whose
/// entry is not a table or whose id is not one is dropped, each recorded
/// as a repair.
pub(super) fn decode_models(
    fields: &mut Fields<'_>,
    index: usize,
    raw: &Value,
) -> BTreeMap<String, AgentModelOverrideSettings> {
    let path = format!("providers[{index}].models");
    let Some(entries) = raw.as_object() else {
        fields.invalid(
            &path,
            Some("the model overrides are not a table and were dropped"),
        );
        return BTreeMap::new();
    };
    let mut models = BTreeMap::new();
    for (model, entry) in entries {
        let entry_path = format!("{path}.\"{model}\"");
        let Some(object) = entry.as_object().filter(|_| is_model_id(model)) else {
            fields.invalid(
                &entry_path,
                Some("an unusable model override was dropped; the others are kept"),
            );
            continue;
        };
        let mut entry = Fields::nested(object, fields.path(&entry_path), fields.repairs());
        let overrides = AgentModelOverrideSettings {
            context_window_tokens: entry.read_checked("contextWindowTokens", None, positive),
            max_output_tokens: entry.read_checked("maxOutputTokens", None, positive),
            image_input: entry.read("imageInput", AgentModelImageInput::Auto),
            reasoning_efforts: entry.read("reasoningEfforts", None),
        };
        models.insert(model.clone(), overrides);
    }
    models
}

fn is_model_id(model: &str) -> bool {
    !model.trim().is_empty() && model.len() <= MAX_MODEL_ID_BYTES
}

fn positive(value: Option<u64>) -> Result<Option<u64>, String> {
    match value {
        Some(0) => Err("must be positive".to_string()),
        value => Ok(value),
    }
}
