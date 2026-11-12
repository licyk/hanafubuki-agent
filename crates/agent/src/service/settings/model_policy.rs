//! The model policy of `providers.toml`, with the default Provider: one
//! record, written whole through the registry's revision-gated call. The
//! default Provider's choices are the registry's Providers, a disabled one
//! with why. A turn's budget (`turnBudget`) is written in the file.

use hanafubuki_agent_config::{
    AgentImageInputPolicy, AgentModelPolicySettings, AgentProviderRegistrySettings,
    AgentRemoteCompactionCapability, SettingChoice, SettingKind, SettingSpec, SettingsGroup,
};
use serde::{Deserialize, Serialize};

use super::record::{merged, record_of};
use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, SAVE,
};
use crate::service::AgentApplicationService;
use crate::AgentModelPolicyWriteRequest;

pub(super) const ID: &str = "modelPolicy";
/// The one record.
const POLICY: &str = "policy";

/// The record's form: the policy, beside the default Provider.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolicyRecord {
    default_provider_id: Option<String>,
    #[serde(flatten)]
    policy: AgentModelPolicySettings,
}

fn fields(registry: &AgentProviderRegistrySettings) -> Vec<SettingSpec> {
    let providers = registry
        .providers
        .iter()
        .map(|provider| SettingChoice {
            disabled: (!provider.enabled).then(|| "The Provider is disabled".to_string()),
            ..SettingChoice::new(&provider.id, &provider.display_name)
        })
        .collect();
    let choice = |values: Vec<(String, &str)>| SettingKind::Choice {
        values: values
            .into_iter()
            .map(|(value, label)| SettingChoice::new(value, label))
            .collect(),
    };
    let integer = |minimum, maximum| SettingKind::Integer { minimum, maximum };
    vec![
        SettingSpec {
            summary: "The Provider new sessions use".into(),
            ..SettingSpec::new(
                "defaultProviderId",
                "Default Provider",
                SettingKind::Choice { values: providers },
            )
        },
        SettingSpec {
            summary: "The model new sessions run".into(),
            ..SettingSpec::new(
                "model",
                "Model",
                SettingKind::Text {
                    maximum_chars: Some(256),
                    secret: false,
                    nullable: false,
                },
            )
        },
        SettingSpec::new(
            "reasoningEffort",
            "Reasoning effort",
            SettingKind::Text {
                maximum_chars: Some(64),
                secret: false,
                nullable: true,
            },
        ),
        SettingSpec::new(
            "inputCapabilities.imageInput",
            "Images",
            choice(vec![
                (name_of(&AgentImageInputPolicy::Auto), "Automatic"),
                (name_of(&AgentImageInputPolicy::Disabled), "Never sent"),
            ]),
        ),
        SettingSpec::new(
            "contextWindowTokens",
            "Context window",
            integer(Some(1), None),
        ),
        SettingSpec::new(
            "autoCompactionEnabled",
            "Automatic compaction",
            SettingKind::Boolean,
        ),
        SettingSpec::new(
            "autoCompactThresholdPercent",
            "Compaction threshold",
            integer(Some(1), Some(100)),
        ),
        SettingSpec::new(
            "remoteCompaction",
            "Remote compaction",
            choice(vec![
                (name_of(&AgentRemoteCompactionCapability::Disabled), "Off"),
                (
                    name_of(&AgentRemoteCompactionCapability::ResponsesCompact),
                    "Responses compact",
                ),
                (
                    name_of(&AgentRemoteCompactionCapability::ResponsesCompactV2),
                    "Responses compact v2",
                ),
            ]),
        ),
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let registry = match service.provider_registry_snapshot().await {
        Ok(registry) => registry,
        Err(error) => {
            return SettingsCollection::new(ID, "Model", SettingsGroup::Model, Vec::new())
                .unavailable(error)
        }
    };
    let fields = fields(&registry);
    let mut record = record_of(POLICY, "Model policy", &policy_record(&registry), &fields);
    record.summary = registry.model_policy.model.clone();
    SettingsCollection {
        records: vec![record],
        revision: Some(registry.revision),
        ..SettingsCollection::new(ID, "Model", SettingsGroup::Model, fields)
            .with_actions(vec![SettingsAction::record(SAVE, "Save")])
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    if request.action != SAVE {
        return request.unknown_action();
    }
    if request.record_id()? != POLICY {
        return Err(format!("the model policy is the record `{POLICY}`"));
    }
    let registry = service.provider_registry_snapshot().await?;
    let written: PolicyRecord = merged(
        &policy_record(&registry),
        &fields(&registry),
        &request.values,
    )?;
    service
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision: request.revision()?,
            default_provider_id: written.default_provider_id,
            policy: written.policy,
        })
        .await?;
    Ok(SettingsActionOutcome::default())
}

fn policy_record(registry: &AgentProviderRegistrySettings) -> PolicyRecord {
    PolicyRecord {
        default_provider_id: registry.default_provider_id.clone(),
        policy: registry.model_policy.clone(),
    }
}
