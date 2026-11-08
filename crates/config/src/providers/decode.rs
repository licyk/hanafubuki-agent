use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{
    validate_agent_model_policy, validate_agent_provider, AgentModelPolicySettings,
    AgentProviderRegistrySettings, AgentProviderSettings, MAX_AGENT_PROVIDERS,
};
use crate::fields::{read_member, read_object, FieldRead, Fields};

/// The registry is repaired per entry: a Provider that cannot be used is
/// dropped alone, and every other Provider keeps its endpoint and
/// credentials.
pub(crate) fn decode_registry(
    object: &Map<String, Value>,
    repairs: &mut Vec<String>,
) -> AgentProviderRegistrySettings {
    let defaults = AgentProviderRegistrySettings::default();
    let mut fields = Fields::new(object, repairs);
    let revision = fields.read("revision", defaults.revision);
    let model_policy = read_object(
        &mut fields,
        "modelPolicy",
        &defaults.model_policy,
        decode_model_policy,
    );
    let providers = decode_providers(&mut fields);
    let default_provider_id = match fields.read_quiet::<Option<String>>("defaultProviderId", Ok) {
        FieldRead::Valid(None) | FieldRead::Missing => None,
        FieldRead::Valid(Some(id)) if is_enabled_provider(&providers, &id) => Some(id),
        FieldRead::Valid(Some(_)) | FieldRead::Invalid => {
            fields.invalid(
                "defaultProviderId",
                Some("the default Provider is not an enabled Provider and was cleared"),
            );
            None
        }
    };
    AgentProviderRegistrySettings {
        revision,
        default_provider_id,
        model_policy,
        providers,
    }
}

fn is_enabled_provider(providers: &[AgentProviderSettings], id: &str) -> bool {
    providers
        .iter()
        .any(|provider| provider.id == id && provider.enabled)
}

fn decode_providers(fields: &mut Fields<'_>) -> Vec<AgentProviderSettings> {
    let Some(raw) = fields.raw("providers") else {
        return Vec::new();
    };
    let Some(items) = raw.as_array() else {
        fields.invalid("providers", None);
        return Vec::new();
    };
    let mut providers = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let provider = serde_json::from_value::<AgentProviderSettings>(item.clone())
            .ok()
            .filter(|provider| validate_agent_provider(provider).is_ok());
        let message = match provider {
            Some(_) if providers.len() >= MAX_AGENT_PROVIDERS => {
                "the Provider registry holds too many Providers; this one was dropped"
            }
            Some(provider) if !ids.insert(provider.id.clone()) => {
                "a second Provider with the same id was dropped"
            }
            Some(provider) => {
                providers.push(provider);
                continue;
            }
            None => "an unusable Provider was dropped; the other Providers are kept",
        };
        fields.invalid(&format!("providers[{index}]"), Some(message));
    }
    providers
}

/// Each model policy setting falls back on its own, checked against the same
/// bounds as the whole policy.
fn decode_model_policy(fields: &mut Fields<'_>, policy: &mut AgentModelPolicySettings) {
    let validate = validate_agent_model_policy;
    read_member(fields, policy, "model", |p| &mut p.model, validate);
    read_member(
        fields,
        policy,
        "reasoningEffort",
        |p| &mut p.reasoning_effort,
        validate,
    );
    read_member(
        fields,
        policy,
        "inputCapabilities",
        |p| &mut p.input_capabilities,
        validate,
    );
    read_member(
        fields,
        policy,
        "contextWindowTokens",
        |p| &mut p.context_window_tokens,
        validate,
    );
    read_member(
        fields,
        policy,
        "autoCompactionEnabled",
        |p| &mut p.auto_compaction_enabled,
        validate,
    );
    read_member(
        fields,
        policy,
        "autoCompactThresholdPercent",
        |p| &mut p.auto_compact_threshold_percent,
        validate,
    );
    read_member(
        fields,
        policy,
        "remoteCompaction",
        |p| &mut p.remote_compaction,
        validate,
    );
    read_member(
        fields,
        policy,
        "turnBudget",
        |p| &mut p.turn_budget,
        validate,
    );
}
