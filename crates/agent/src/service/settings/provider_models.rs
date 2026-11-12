//! The per-model overrides of the Providers (`models."<model>"` of a
//! Provider in `providers.toml`): a record per Provider and model
//! (`PROVIDER/MODEL`, split at the first `/`, a model id holding more),
//! written through the registry's revision gate. Each fact set beats what
//! is known of the model; an unset one leaves the others to say.

use hanafubuki_agent_config::{
    AgentModelImageInput, AgentModelOverrideSettings, SettingChoice, SettingKind, SettingSpec,
    SettingsGroup,
};
use serde_json::Value;

use super::record::{merged, record_of, with_defaults};
use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD,
    DELETE, SAVE,
};
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "providerModels";
/// The longest model id, as the registry's reader takes it.
const MAX_MODEL_ID_BYTES: usize = 256;

fn fields() -> Vec<SettingSpec> {
    let tokens = |key: &str, label: &str, summary: &str| SettingSpec {
        summary: summary.into(),
        ..SettingSpec::new(
            key,
            label,
            SettingKind::Integer {
                minimum: Some(1),
                maximum: None,
            },
        )
    };
    let image =
        |input: AgentModelImageInput, label: &str| SettingChoice::new(name_of(&input), label);
    vec![
        tokens(
            "contextWindowTokens",
            "Context window",
            "In tokens; beats the model policy's window for this model",
        ),
        tokens("maxOutputTokens", "Maximum output", "In tokens"),
        SettingSpec::new(
            "imageInput",
            "Image input",
            SettingKind::Choice {
                values: vec![
                    image(AgentModelImageInput::Auto, "As known"),
                    image(AgentModelImageInput::Supported, "Supported"),
                    image(AgentModelImageInput::Unsupported, "Not supported"),
                ],
            },
        ),
        SettingSpec {
            summary: "The levels it takes, in place of the known ones; empty: none".into(),
            ..SettingSpec::new(
                "reasoningEfforts",
                "Reasoning levels",
                SettingKind::TextList {
                    maximum_items: None,
                    maximum_item_chars: None,
                },
            )
        },
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let fields = with_defaults(fields(), &as_record(&AgentModelOverrideSettings::default()));
    let collection = SettingsCollection::new(ID, "Model overrides", SettingsGroup::Model, fields)
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a model")
                .named("Provider and model (PROVIDER/MODEL)"),
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record(DELETE, "Delete").confirmed(),
        ]);
    let registry = match service.provider_registry_snapshot().await {
        Ok(registry) => registry,
        Err(error) => return collection.unavailable(error),
    };
    let fields = &collection.record;
    let records = registry
        .providers
        .iter()
        .flat_map(|provider| {
            provider.models.iter().map(move |(model, overrides)| {
                let id = format!("{}/{model}", provider.id);
                let label = format!("{} · {model}", provider.display_name);
                let mut record = record_of(&id, &label, &as_record(overrides), fields);
                record.summary = format!("{} settings", record.values.len());
                record
            })
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
    let (provider_id, model) = parse(request.record_id()?)?;
    let registry = service.provider_registry_snapshot().await?;
    let provider = registry
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| format!("there is no Provider `{provider_id}`"))?;
    let current = provider.models.get(&model);
    let overrides = match (request.action.as_str(), current) {
        (ADD, Some(_)) => return Err(format!("`{model}` of `{provider_id}` has its settings")),
        (ADD, None) => Some(merged(
            &AgentModelOverrideSettings::default(),
            &fields(),
            &request.values,
        )?),
        (SAVE, Some(current)) => Some(merged(current, &fields(), &request.values)?),
        (DELETE, Some(_)) => None,
        (SAVE | DELETE, None) => {
            return Err(format!("`{model}` of `{provider_id}` has no settings"))
        }
        _ => return request.unknown_action(),
    };
    let added = (request.action == ADD).then(|| format!("{provider_id}/{model}"));
    service
        .write_provider_model(request.revision()?, provider_id, model, overrides)
        .await?;
    Ok(SettingsActionOutcome {
        record: added,
        message: None,
    })
}

/// `overrides` serialized with its image input, which the file leaves out
/// when it is `auto`.
fn as_record(overrides: &AgentModelOverrideSettings) -> Value {
    let mut record = serde_json::to_value(overrides).unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut record {
        fields
            .entry("imageInput")
            .or_insert_with(|| Value::String(name_of(&overrides.image_input)));
    }
    record
}

/// `PROVIDER/MODEL`: the model id is what follows the first `/`, and the
/// registry's reader keeps it when it is not blank and at most
/// [`MAX_MODEL_ID_BYTES`] long.
fn parse(record: &str) -> Result<(String, String), String> {
    let (provider, model) = record
        .split_once('/')
        .ok_or_else(|| format!("`{record}` is not PROVIDER/MODEL"))?;
    let model = model.trim();
    if model.is_empty() || model.len() > MAX_MODEL_ID_BYTES {
        return Err(format!(
            "a model id has 1 to {MAX_MODEL_ID_BYTES} bytes, not `{model}`"
        ));
    }
    Ok((provider.trim().to_string(), model.to_string()))
}
