//! The profiles of the user's `config.toml` (`[profiles.<name>]`), the
//! standalone Agent's: each one's settings, read and written by the specs
//! of `config.toml` (as `config set profiles.<name>.<key>` writes them). A
//! key is removed from a profile, and a profile deleted, in the file.

use hanafubuki_agent_config::{config_sections, SettingSpec, SettingsGroup};
use serde_json::{Map, Value};

use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD, SAVE,
};
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "profiles";
const PROFILES: &str = "profiles";

fn fields() -> Vec<SettingSpec> {
    config_sections()
        .into_iter()
        .flat_map(|section| section.settings)
        .collect()
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Profiles", SettingsGroup::Advanced, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a profile"),
            SettingsAction::record(SAVE, "Save"),
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
    if !matches!(request.action.as_str(), ADD | SAVE) {
        return request.unknown_action();
    }
    let name = request.record_id()?.to_string();
    if request.values.is_empty() {
        return Err("a profile's save names the settings it writes".to_string());
    }
    let mut changes = Vec::new();
    for (key, value) in &request.values {
        if !fields().iter().any(|spec| spec.key == *key) {
            return Err(format!("`{key}` is not a setting of a profile"));
        }
        if value.is_null() {
            return Err(format!(
                "`{key}` is removed from the profile `{name}` in the file"
            ));
        }
        changes.push((format!("{PROFILES}.{}.{key}", quoted(&name)), value.clone()));
    }
    let config = service.config.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent profile write worker",
        move || {
            changes
                .into_iter()
                .try_for_each(|(key, value)| config.set_user_json(&key, value))
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
