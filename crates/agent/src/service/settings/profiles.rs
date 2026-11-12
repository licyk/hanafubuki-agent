//! The profiles of the user's `config.toml` (`[profiles.<name>]`), the
//! standalone Agent's: each one's settings, read and written by the specs
//! of `config.toml` (as `config set profiles.<name>.<key>` writes them). A
//! save of `null` removes a key from the profile; a profile is deleted
//! whole, unless it is the one the Agent runs with.

use hanafubuki_agent_config::{
    config_sections, ConfigLayerSource, SettingSpec, SettingsGroup, UserEntry,
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
fn fields() -> Vec<SettingSpec> {
    config_sections()
        .into_iter()
        .flat_map(|section| section.settings)
        .map(|spec| SettingSpec {
            default: None,
            ..spec
        })
        .collect()
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Profiles", SettingsGroup::Advanced, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a profile").named("Profile name"),
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record(DELETE, "Delete").confirmed(),
        ]);
    let profiles = match user_table(service, PROFILES).await {
        Ok(profiles) => profiles,
        Err(error) => return collection.unavailable(error),
    };
    let fields = fields();
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
    let mut changes = Vec::new();
    let mut removed = Map::new();
    for (key, value) in &request.values {
        if !fields().iter().any(|spec| spec.key == *key) {
            return Err(format!("`{key}` is not a setting of a profile"));
        }
        if value.is_null() {
            removed.insert(key.clone(), Value::Null);
        } else {
            changes.push((format!("{PROFILES}.{}.{key}", quoted(&name)), value.clone()));
        }
    }
    let profile = name.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent profile write worker",
        move || {
            for (key, value) in changes {
                config.set_user_json(&key, value)?;
            }
            if removed.is_empty() {
                return Ok(());
            }
            config.write_entry_values(UserEntry::Profile(&profile), &removed)
        },
    )
    .await??;
    service.apply_settings_to_plugins();
    Ok(SettingsActionOutcome {
        record: (request.action == ADD).then_some(name),
        message: None,
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
