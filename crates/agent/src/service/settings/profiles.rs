//! The profiles of the user's `config.toml` (`[profiles.<name>]`), the
//! standalone Agent's: each one's settings, read and written by the specs
//! of `config.toml` and the plugins' registered option schemas. A
//! save of `null` removes a key from the profile; a profile is deleted
//! whole, unless it is the one the Agent runs with.

use hanafubuki_agent_config::{
    config_sections, ConfigLayerSource, SettingSpec, SettingsGroup, SettingsLayer, UserEntry,
};
use serde_json::{Map, Value};

use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD, DELETE,
    SAVE,
};
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "profiles";
const PROFILES: &str = "profiles";

/// The settings of `config.toml`, without their defaults: a profile sets
/// only what it names, and the rest comes from below it.
pub(super) async fn fields(service: &AgentApplicationService) -> Vec<SettingSpec> {
    let mut fields = config_sections()
        .into_iter()
        .flat_map(|section| section.settings)
        .chain(
            service
                .host
                .option_schemas()
                .into_iter()
                .flat_map(|(row, schema)| {
                    schema.options.into_iter().map(move |spec| SettingSpec {
                        key: format!("pluginOptions.{row}.{}", spec.key),
                        ..spec
                    })
                }),
        )
        .map(|spec| SettingSpec {
            default: None,
            ..spec
        })
        .collect();
    super::layer_fields::extend(service, &mut fields).await;
    fields
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let fields = fields(service).await;
    let collection =
        SettingsCollection::new(ID, "Profiles", SettingsGroup::Advanced, fields.clone())
            .with_actions(vec![
                SettingsAction::collection(ADD, "Add a profile").named("Profile name"),
                SettingsAction::record(SAVE, "Save"),
                SettingsAction::record(DELETE, "Delete").confirmed(),
            ]);
    let profiles = match user_table(service, PROFILES).await {
        Ok(profiles) => profiles,
        Err(error) => return collection.unavailable(error),
    };
    let records = profiles
        .iter()
        .map(|(name, profile)| {
            let mut record = record_of(name, name, profile, &fields);
            record.summary = format!("{} settings", record.values.len());
            record
        })
        .collect();
    SettingsCollection {
        records,
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let name = request.record_id()?.to_string();
    let config = service.config.clone();
    match request.action.as_str() {
        ADD | SAVE => {}
        DELETE => {
            hanafubuki_agent_foundation::blocking::run_blocking_operation(
                service.blocking.as_ref(),
                "agent profile delete worker",
                move || {
                    let selected = config.layer_sources()?.into_iter().any(|source| {
                        matches!(source, ConfigLayerSource::Profile { name: selected } if selected == name)
                    });
                    if selected {
                        return Err(format!(
                            "the profile `{name}` is the one this Agent runs with"
                        ));
                    }
                    config.remove_entry(UserEntry::Profile(&name))
                },
            )
            .await??;
            service.apply_settings_to_plugins();
            return Ok(SettingsActionOutcome::default());
        }
        _ => return request.unknown_action(),
    }
    if request.values.is_empty() {
        return Err("a profile's save names the settings it writes".to_string());
    }
    let fields = fields(service).await;
    let message = super::layer_fields::write(
        service,
        SettingsLayer::Profile(name.clone()),
        request.values,
        fields,
    )
    .await?;
    Ok(SettingsActionOutcome {
        record: (request.action == ADD).then_some(name),
        message: Some(message),
    })
}

/// The table `table` of the user's `config.toml`, each entry that is one.
pub(super) async fn user_table(
    service: &AgentApplicationService,
    table: &'static str,
) -> Result<Vec<(String, Value)>, String> {
    let config = service.config.clone();
    let document = hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent user settings read worker",
        move || config.user_document(),
    )
    .await??;
    let entries = document
        .get(table)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);
    Ok(entries
        .into_iter()
        .filter(|(_, entry)| entry.is_object())
        .collect())
}

/// `name` as one key of a dotted TOML key, quoted: a JSON string is a TOML
/// basic string.
pub(super) fn quoted(name: &str) -> String {
    serde_json::Value::String(name.to_string()).to_string()
}
