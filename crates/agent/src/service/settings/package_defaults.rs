//! The enablement inherited by installed plugins without their own override.

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingValue, SettingsGroup};
use serde_json::json;

use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, SAVE,
};
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "packageDefaults";
const RECORD: &str = "default";

fn fields() -> Vec<SettingSpec> {
    vec![SettingSpec {
        summary: "Used when an installed plugin has no enabled override; a disabled marketplace still keeps its plugins off".into(),
        default: Some(SettingValue::Boolean(true)),
        ..SettingSpec::new("enabled", "Enable plugins by default", SettingKind::Boolean)
    }]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let fields = fields();
    let collection = SettingsCollection::new(
        ID,
        "Installed plugin defaults",
        SettingsGroup::Plugins,
        fields.clone(),
    )
    .with_actions(vec![SettingsAction::record(SAVE, "Save")]);
    let settings = match service.settings_view().await {
        Ok(settings) => settings.settings,
        Err(error) => return collection.unavailable(error),
    };
    let enabled = settings
        .plugins
        .default
        .is_none_or(|default| default.enabled);
    SettingsCollection {
        records: vec![record_of(
            RECORD,
            "Default",
            &json!({"enabled": enabled}),
            &fields,
        )],
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    if request.action != SAVE {
        return request.unknown_action();
    }
    if request.record_id()? != RECORD {
        return Err("there is no such installed plugin defaults record".into());
    }
    if request.values.is_empty() {
        return Ok(SettingsActionOutcome::default());
    }
    let value = request
        .values
        .get("enabled")
        .filter(|_| request.values.len() == 1)
        .ok_or("installed plugin defaults only has an enabled field")?;
    let enabled = fields()[0].check(value)?.to_json();
    let config = service.config.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent installed plugin defaults write worker",
        move || config.set_user_json("plugins._default.enabled", enabled),
    )
    .await??;
    service.reload_packages().await?;
    Ok(SettingsActionOutcome::default())
}
