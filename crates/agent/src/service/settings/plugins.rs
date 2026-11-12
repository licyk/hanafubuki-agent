//! The Agent's plugin rows: each one's state, turned on or off through
//! `disabledPlugins`, and started again after a failure. A row's options
//! are a section of their own (the Plugins group).

use std::collections::BTreeMap;

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingValue, SettingsGroup};

use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection,
    SettingsRecord, SAVE,
};
use crate::host::AgentPluginStatus;
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "plugins";
const RETRY: &str = "retry";
const ENABLED: &str = "enabled";

fn fields() -> Vec<SettingSpec> {
    vec![SettingSpec::new(ENABLED, "Enabled", SettingKind::Boolean)]
}

pub(super) fn collection(statuses: &[AgentPluginStatus]) -> SettingsCollection {
    let records = statuses
        .iter()
        .map(|status| SettingsRecord {
            id: status.name.clone(),
            label: status.name.clone(),
            summary: status.summary.clone(),
            state: Some(name_of(&status.state)),
            issue: status.error.clone().or_else(|| status.warning.clone()),
            read_only: None,
            values: BTreeMap::from([(ENABLED.to_string(), SettingValue::Boolean(status.enabled))]),
            secrets_set: Vec::new(),
        })
        .collect();
    SettingsCollection {
        records,
        ..SettingsCollection::new(ID, "Plugins", SettingsGroup::Plugins, fields()).with_actions(
            vec![
                SettingsAction::record(SAVE, "Save"),
                SettingsAction::record(RETRY, "Retry"),
            ],
        )
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let name = request.record_id()?.to_string();
    match request.action.as_str() {
        SAVE => {
            for (key, value) in &request.values {
                let enabled = match (key.as_str(), value.as_bool()) {
                    (ENABLED, Some(enabled)) => enabled,
                    (ENABLED, None) => return Err("`enabled` is true or false".to_string()),
                    _ => return Err(format!("`{key}` is not a field of this record")),
                };
                service.set_plugin_enabled(name.clone(), enabled).await?;
            }
            Ok(SettingsActionOutcome::default())
        }
        RETRY => {
            service.retry_plugin(&name).await?;
            Ok(SettingsActionOutcome::default())
        }
        _ => request.unknown_action(),
    }
}
