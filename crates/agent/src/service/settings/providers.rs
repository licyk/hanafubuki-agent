//! The Providers of `providers.toml`: the registry's records, written
//! through its revision-gated calls. The API token is a secret: the catalog
//! says only whether it is set, and a save that leaves it out keeps it.

use std::collections::BTreeMap;

use hanafubuki_agent_config::{
    AgentModelProtocol, AgentProviderSettings, SettingChoice, SettingKind, SettingSpec,
    SettingsGroup,
};

use super::record::{merged, record_of};
use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD,
    DELETE, SAVE,
};
use crate::service::AgentApplicationService;
use crate::{
    AgentModelPolicyWriteRequest, AgentProviderDeleteRequest, AgentProviderDraft,
    AgentProviderModelCatalogRequest, AgentProviderWriteRequest,
};

pub(super) const ID: &str = "providers";
const SET_DEFAULT: &str = "set_default";
const LIST_MODELS: &str = "list_models";
/// The models a listing names in its message; the rest are counted.
const LISTED_MODELS: usize = 10;

fn fields() -> Vec<SettingSpec> {
    let text = |maximum_chars, secret, nullable| SettingKind::Text {
        maximum_chars,
        secret,
        nullable,
    };
    let protocol =
        |protocol: AgentModelProtocol, label: &str| SettingChoice::new(name_of(&protocol), label);
    vec![
        SettingSpec::new("displayName", "Name", text(Some(128), false, false)),
        SettingSpec::new("enabled", "Enabled", SettingKind::Boolean),
        SettingSpec::new(
            "protocol",
            "Protocol",
            SettingKind::Choice {
                values: vec![
                    protocol(AgentModelProtocol::Responses, "Responses"),
                    protocol(AgentModelProtocol::ChatCompletions, "Chat Completions"),
                    protocol(AgentModelProtocol::AnthropicMessages, "Anthropic Messages"),
                ],
            },
        ),
        SettingSpec {
            summary: "The endpoint; the protocol's path is appended to it".into(),
            ..SettingSpec::new("baseUrl", "Base URL", text(None, false, false))
        },
        SettingSpec::new("apiToken", "API token", text(None, true, true)),
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Providers", SettingsGroup::Model, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a Provider"),
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record(DELETE, "Delete").confirmed(),
            SettingsAction::record(SET_DEFAULT, "Make default"),
            SettingsAction::record(LIST_MODELS, "List models"),
        ]);
    let registry = match service.provider_registry_snapshot().await {
        Ok(registry) => registry,
        Err(error) => return collection.unavailable(error),
    };
    let fields = fields();
    let default = registry.default_provider_id.as_deref();
    let records = registry
        .providers
        .iter()
        .map(|provider| {
            let mut record = record_of(&provider.id, &provider.display_name, provider, &fields);
            record.summary = format!("{} · {}", name_of(&provider.protocol), provider.base_url);
            record.state = if default == Some(provider.id.as_str()) {
                Some("default".into())
            } else {
                (!provider.enabled).then(|| "off".into())
            };
            record
        })
        .collect();
    SettingsCollection {
        records,
        revision: Some(registry.revision),
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    match request.action.as_str() {
        ADD => add(service, &request).await,
        SAVE => {
            let id = request.record_id()?;
            let registry = service.provider_registry_snapshot().await?;
            let current = registry
                .providers
                .iter()
                .find(|provider| provider.id == id)
                .ok_or_else(|| format!("there is no Provider `{id}`"))?;
            let provider: AgentProviderSettings = merged(current, &fields(), &request.values)?;
            write(service, &request, Some(id.to_string()), provider).await?;
            Ok(SettingsActionOutcome::default())
        }
        DELETE => {
            service
                .delete_provider(AgentProviderDeleteRequest {
                    expected_revision: request.revision()?,
                    provider_id: request.record_id()?.to_string(),
                })
                .await?;
            Ok(SettingsActionOutcome::default())
        }
        SET_DEFAULT => {
            let registry = service.provider_registry_snapshot().await?;
            service
                .write_model_policy(AgentModelPolicyWriteRequest {
                    expected_revision: request.revision()?,
                    default_provider_id: Some(request.record_id()?.to_string()),
                    policy: registry.model_policy,
                })
                .await?;
            Ok(SettingsActionOutcome::default())
        }
        LIST_MODELS => list_models(service, request.record_id()?).await,
        _ => request.unknown_action(),
    }
}

/// A new Provider from `request.values` over an empty, enabled one; the
/// registry names it.
async fn add(
    service: &AgentApplicationService,
    request: &SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let template = AgentProviderSettings {
        id: String::new(),
        display_name: String::new(),
        enabled: true,
        protocol: AgentModelProtocol::Responses,
        base_url: String::new(),
        api_token: None,
        request_headers: Vec::new(),
        models: BTreeMap::new(),
    };
    let provider: AgentProviderSettings = merged(&template, &fields(), &request.values)?;
    let before = service.provider_registry_snapshot().await?;
    let after = write(service, request, None, provider).await?;
    let added = after
        .providers
        .iter()
        .find(|provider| !before.providers.iter().any(|old| old.id == provider.id))
        .map(|provider| provider.id.clone());
    Ok(SettingsActionOutcome {
        record: added,
        message: None,
    })
}

async fn write(
    service: &AgentApplicationService,
    request: &SettingsActionRequest,
    provider_id: Option<String>,
    provider: AgentProviderSettings,
) -> Result<crate::AgentProviderRegistrySnapshot, String> {
    service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: request.revision()?,
            provider_id,
            provider: AgentProviderDraft {
                display_name: provider.display_name,
                enabled: provider.enabled,
                protocol: provider.protocol,
                base_url: provider.base_url,
                api_token: provider.api_token,
                request_headers: provider.request_headers,
            },
        })
        .await
}

async fn list_models(
    service: &AgentApplicationService,
    id: &str,
) -> Result<SettingsActionOutcome, String> {
    let listing = service
        .list_provider_models(AgentProviderModelCatalogRequest {
            provider_ids: Some(vec![id.to_string()]),
        })
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| format!("there is no Provider `{id}`"))?;
    if let Some(error) = listing.error {
        return Err(error);
    }
    let names = listing
        .models
        .iter()
        .take(LISTED_MODELS)
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let more = listing.models.len().saturating_sub(LISTED_MODELS);
    let message = match (listing.models.len(), more) {
        (0, _) => "no models".to_string(),
        (count, 0) => format!("{count} models: {names}"),
        (count, more) => format!("{count} models: {names} and {more} more"),
    };
    Ok(SettingsActionOutcome {
        record: None,
        message: Some(message),
    })
}
