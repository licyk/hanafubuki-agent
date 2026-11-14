//! The model policy of `providers.toml`, with the default Provider: one
//! record, written whole through the registry's revision-gated call. The
//! default Provider's choices are the registry's Providers, a disabled one
//! with why. A turn's budget (`turnBudget`) is the Agent's unless
//! `customTurnBudget` is on; its fields start from the Agent's.

use hanafubuki_agent_config::{
    AgentImageInputPolicy, AgentModelPolicySettings, AgentProviderRegistrySettings,
    AgentRemoteCompactionCapability, AgentTurnBudgetSettings, SettingChoice, SettingCondition,
    SettingKind, SettingSpec, SettingValue, SettingsGroup, MAX_AGENT_CONSECUTIVE_STALLED_STEPS,
    MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS, MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS,
    MAX_AGENT_HOOK_FRAGMENT_BYTES, MAX_AGENT_IDLE_TIMEOUT_SECONDS, MAX_AGENT_IN_FLIGHT_TOOLS,
    MAX_AGENT_PENDING_INPUT_ITEMS, MAX_AGENT_TOOL_INVOCATIONS, MAX_AGENT_TOOL_OUTPUT_BYTES,
};
use hanafubuki_agent_foundation::agent_settings::MAX_MODEL_REASONING_EFFORT_BYTES;
use serde::{Deserialize, Serialize};

use super::record::{merged, record_of};
use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, SAVE,
};
use crate::service::AgentApplicationService;
use crate::{AgentModelPolicyWriteRequest, KnownReasoning, ModelMetadata};

pub(super) const ID: &str = "modelPolicy";
/// The one record.
const POLICY: &str = "policy";
const CUSTOM_BUDGET: &str = "customTurnBudget";
/// The turn budget's fields: key, label and largest value.
const BUDGET: [(&str, &str, u64); 11] = [
    (
        "maximumCumulativeModelInputTokens",
        "Input tokens",
        MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS,
    ),
    (
        "maximumCumulativeModelOutputTokens",
        "Output tokens",
        MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS,
    ),
    (
        "maximumToolInvocations",
        "Tool calls",
        MAX_AGENT_TOOL_INVOCATIONS,
    ),
    (
        "maximumToolOutputBytes",
        "Tool output bytes",
        MAX_AGENT_TOOL_OUTPUT_BYTES,
    ),
    (
        "maximumHookFragmentBytes",
        "Hook fragment bytes",
        MAX_AGENT_HOOK_FRAGMENT_BYTES,
    ),
    (
        "maximumPendingInputItems",
        "Pending inputs",
        MAX_AGENT_PENDING_INPUT_ITEMS as u64,
    ),
    (
        "maximumInFlightTools",
        "Tools at once",
        MAX_AGENT_IN_FLIGHT_TOOLS as u64,
    ),
    (
        "maximumConsecutiveStalledSteps",
        "Stalled steps",
        MAX_AGENT_CONSECUTIVE_STALLED_STEPS as u64,
    ),
    (
        "modelStreamIdleTimeoutSeconds",
        "Model idle seconds",
        MAX_AGENT_IDLE_TIMEOUT_SECONDS,
    ),
    (
        "toolIdleTimeoutSeconds",
        "Tool idle seconds",
        MAX_AGENT_IDLE_TIMEOUT_SECONDS,
    ),
    (
        "approvalTimeoutSeconds",
        "Approval seconds",
        MAX_AGENT_IDLE_TIMEOUT_SECONDS,
    ),
];

/// The record's form: the policy, beside the default Provider.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolicyRecord {
    default_provider_id: Option<String>,
    custom_turn_budget: bool,
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
            effort_kind(Vec::new()),
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
        SettingSpec {
            advanced: true,
            summary: "Off, a turn has the Agent's limits".into(),
            ..SettingSpec::new(CUSTOM_BUDGET, "Custom turn budget", SettingKind::Boolean)
        },
    ]
    .into_iter()
    .chain(BUDGET.iter().map(|(key, label, maximum)| SettingSpec {
        advanced: true,
        visible_if: Some(SettingCondition {
            key: CUSTOM_BUDGET.into(),
            equals: SettingValue::Boolean(true),
        }),
        ..SettingSpec::new(
            format!("turnBudget.{key}"),
            *label,
            integer(Some(1), Some(*maximum as i64)),
        )
    }))
    .collect()
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let registry = match service.provider_registry_snapshot().await {
        Ok(registry) => registry,
        Err(error) => {
            return SettingsCollection::new(ID, "Model", SettingsGroup::Model, Vec::new())
                .unavailable(error)
        }
    };
    let mut fields = fields(&registry);
    if let Some(provider_id) = registry.default_provider_id.as_deref() {
        let metadata = service
            .model_metadata(provider_id, &registry.model_policy.model)
            .await
            .unwrap_or_default();
        describe_known(&mut fields, &metadata);
    }
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
    let mut written: PolicyRecord = merged(
        &policy_record(&registry),
        &fields(&registry),
        &request.values,
    )?;
    if !written.custom_turn_budget {
        written.policy.turn_budget = None;
    }
    service
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision: request.revision()?,
            default_provider_id: written.default_provider_id,
            policy: written.policy,
        })
        .await?;
    Ok(SettingsActionOutcome::default())
}

/// What is known of the model, said beside the fields it informs: the
/// efforts it reasons at, offered as the effort's choices, and the window
/// it has when none is set.
fn describe_known(fields: &mut [SettingSpec], metadata: &ModelMetadata) {
    for spec in fields {
        match spec.key.as_str() {
            "reasoningEffort" => {
                let known = metadata.known_reasoning();
                spec.summary = match &known {
                    KnownReasoning::Levels { efforts, .. } => {
                        let efforts = efforts
                            .iter()
                            .map(|effort| effort.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("The model takes {efforts}; unset, the Provider's default")
                    }
                    KnownReasoning::Unsupported { learned: true } => {
                        "The Provider refused the reasoning parameter".into()
                    }
                    KnownReasoning::Unsupported { learned: false } => {
                        "The model takes no reasoning effort".into()
                    }
                    KnownReasoning::Unknown => "Unset, the Provider's default".into(),
                };
                spec.kind = effort_kind(reasoning_effort_choices(&known));
            }
            "contextWindowTokens" => {
                spec.summary = match &metadata.context_window {
                    Some(window) => format!(
                        "Unset, the model's: {} ({})",
                        window.value,
                        window.source.label()
                    ),
                    None => "Unset, it is unknown: compacts on the Provider's error".into(),
                }
            }
            _ => {}
        }
    }
}

/// The levels an effort is chosen from, the model's default marked: those
/// `known` names, none when it names none. `/effort` and the settings
/// center's row offer them, beside the Provider's default and a typed one.
pub fn reasoning_effort_choices(known: &KnownReasoning) -> Vec<SettingChoice> {
    let KnownReasoning::Levels { efforts, default } = known else {
        return Vec::new();
    };
    efforts
        .iter()
        .map(|effort| {
            let label = if Some(effort) == default.as_ref() {
                format!("{effort} (model default)")
            } else {
                effort.to_string()
            };
            SettingChoice::new(effort.as_str(), label)
        })
        .collect()
}

/// An effort: one of `values`, one typed (checked as an effort when the
/// record is written), or unset.
fn effort_kind(values: Vec<SettingChoice>) -> SettingKind {
    SettingKind::ChoiceOrText {
        values,
        maximum_chars: Some(MAX_MODEL_REASONING_EFFORT_BYTES as u32),
        nullable: true,
    }
}

/// The record of `registry`'s policy; without a turn budget of its own,
/// the budget's fields hold the Agent's.
fn policy_record(registry: &AgentProviderRegistrySettings) -> PolicyRecord {
    let mut policy = registry.model_policy.clone();
    let custom_turn_budget = policy.turn_budget.is_some();
    policy.turn_budget.get_or_insert_with(agent_budget);
    PolicyRecord {
        default_provider_id: registry.default_provider_id.clone(),
        custom_turn_budget,
        policy,
    }
}

fn agent_budget() -> AgentTurnBudgetSettings {
    let budget = hanafubuki_agent_kernel::AgentTurnBudgetPolicy::default();
    AgentTurnBudgetSettings {
        maximum_cumulative_model_input_tokens: budget.maximum_cumulative_model_input_tokens,
        maximum_cumulative_model_output_tokens: budget.maximum_cumulative_model_output_tokens,
        maximum_tool_invocations: budget.maximum_tool_invocations,
        maximum_tool_output_bytes: budget.maximum_tool_output_bytes,
        maximum_hook_fragment_bytes: budget.maximum_hook_fragment_bytes,
        maximum_pending_input_items: budget.maximum_pending_input_items,
        maximum_in_flight_tools: budget.maximum_in_flight_tools,
        maximum_consecutive_stalled_steps: budget.maximum_consecutive_stalled_steps,
        model_stream_idle_timeout_seconds: budget.model_stream_idle_timeout_seconds,
        tool_idle_timeout_seconds: budget.tool_idle_timeout_seconds,
        approval_timeout_seconds: budget.approval_timeout_seconds,
    }
}
