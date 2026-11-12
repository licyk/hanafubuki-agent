//! A code plugin's settings (`[plugins."<id>"].settings`) as a section of
//! the Plugins group: one per installed package whose `configSchema`
//! lowers to options, its keys the options' full keys
//! (`plugins."<id>".settings.<option>`). A write of them is the package's
//! own: its settings as the user's file holds them, with the changes made,
//! checked against the options and the schema, then the guest starts again
//! with them ([`AgentApplicationService::set_package_settings`]).

use hanafubuki_agent_config::{
    SettingSource, SettingSpec, SettingState, SettingWriteOp, SettingsGroup, SettingsSection,
    SettingsWrite, SettingsWriteOutcome,
};
use serde_json::{Map, Value};

use super::profiles::quoted;
use crate::packages::AgentPackageStatus;
use crate::service::AgentApplicationService;

/// Where the package sections come after the built-in plugins' options.
const FIRST_ORDER: usize = 1000;

/// The section of each package whose options its schema describes.
pub(super) fn sections(packages: &[AgentPackageStatus]) -> Vec<SettingsSection> {
    packages
        .iter()
        .filter_map(|package| {
            let options = package.guest.as_ref()?.options.as_ref()?;
            (!options.schema.options.is_empty()).then_some((package, options))
        })
        .enumerate()
        .map(|(index, (package, options))| {
            let settings = options
                .schema
                .options
                .iter()
                .map(|spec| SettingSpec {
                    key: package_setting_key(&package.id, &spec.key),
                    ..spec.clone()
                })
                .collect();
            let values = options
                .values
                .iter()
                .map(|option| {
                    let mention = format!("`{}`", option.key);
                    SettingState {
                        key: package_setting_key(&package.id, &option.key),
                        value: option.value.clone(),
                        source: match option.layer {
                            Some(_) => SettingSource::User,
                            None => SettingSource::Default,
                        },
                        issue: options
                            .issues
                            .iter()
                            .find(|issue| issue.contains(&mention))
                            .cloned(),
                    }
                })
                .collect();
            SettingsSection {
                id: prefix(&package.id),
                title: package.display_name.clone(),
                group: SettingsGroup::Plugins,
                order: u16::try_from(FIRST_ORDER + index * 10).unwrap_or(u16::MAX),
                settings,
                values,
            }
        })
        .collect()
}

/// `write` when its keys are a package's settings: `None` when none is;
/// refused when it mixes them with other settings, or two packages'.
pub(super) async fn write(
    service: &AgentApplicationService,
    write: &SettingsWrite,
) -> Option<Result<SettingsWriteOutcome, String>> {
    if !write
        .changes
        .iter()
        .any(|change| change.key.starts_with("plugins."))
    {
        return None;
    }
    let packages = match service.packages().await {
        Ok(packages) => packages,
        Err(error) => return Some(Err(error)),
    };
    let owner = |key: &str| {
        packages
            .iter()
            .find(|package| key.starts_with(&format!("{}.", prefix(&package.id))))
    };
    if write
        .changes
        .iter()
        .all(|change| owner(&change.key).is_none())
    {
        return None;
    }
    let first = owner(&write.changes[0].key);
    Some(match first {
        Some(package)
            if write
                .changes
                .iter()
                .all(|change| owner(&change.key).map(|owner| &owner.id) == Some(&package.id)) =>
        {
            write_package(service, package, write).await
        }
        _ => Err("a plugin's settings are written on their own".to_string()),
    })
}

async fn write_package(
    service: &AgentApplicationService,
    package: &AgentPackageStatus,
    write: &SettingsWrite,
) -> Result<SettingsWriteOutcome, String> {
    let id = package.id.as_str();
    let options = package
        .guest
        .as_ref()
        .and_then(|guest| guest.options.as_ref())
        .map(|options| options.schema.options.as_slice())
        .unwrap_or_default();
    let mut settings = written(service, id).await?;
    let mut outcome = SettingsWriteOutcome {
        written: Vec::new(),
        shadowed: Vec::new(),
        apply: Vec::new(),
    };
    for change in &write.changes {
        let spec = options
            .iter()
            .find(|spec| package_setting_key(id, &spec.key) == change.key)
            .ok_or_else(|| format!("the plugin {id} has no setting `{}`", change.key))?;
        match &change.op {
            SettingWriteOp::Set(value) => {
                let value = spec
                    .check(value)
                    .map_err(|error| format!("`{}` cannot take this value: {error}", change.key))?;
                settings.insert(spec.key.clone(), value.to_json());
            }
            SettingWriteOp::Reset => {
                settings.remove(&spec.key);
            }
        }
        outcome.written.push(change.key.clone());
        outcome.apply.push((change.key.clone(), spec.apply));
    }
    let settings = if settings.is_empty() {
        Value::Null
    } else {
        Value::Object(settings)
    };
    service.set_package_settings(id, settings).await?;
    Ok(outcome)
}

/// The settings of `id` as the user's file holds them.
async fn written(
    service: &AgentApplicationService,
    id: &str,
) -> Result<Map<String, Value>, String> {
    let config = service.config.clone();
    let document = hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent plugin settings read worker",
        move || config.user_document(),
    )
    .await??;
    Ok(document
        .get("plugins")
        .and_then(|plugins| plugins.get(id))
        .and_then(|entry| entry.get("settings"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default())
}

/// `plugins."<id>".settings`.
fn prefix(id: &str) -> String {
    format!("plugins.{}.settings", quoted(id))
}

/// The key a write of settings names the setting `option` of the code of
/// the installed plugin `id` by (`plugins."<id>".settings.<option>`), the
/// option quoted when it is not a bare TOML key.
pub fn package_setting_key(id: &str, option: &str) -> String {
    let bare = !option.is_empty()
        && option
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        format!("{}.{option}", prefix(id))
    } else {
        format!("{}.{}", prefix(id), quoted(option))
    }
}
