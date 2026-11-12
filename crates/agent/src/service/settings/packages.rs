//! The installed plugins (packages): each one turned on or off, installed
//! from an added marketplace (`PLUGIN@MARKETPLACE`) or a directory,
//! updated, loaded again, its hooks trusted and uninstalled (keeping its
//! data or not), through the package calls. A code plugin's settings are
//! a section of their own (the Plugins group); the grant of what its code
//! asks for is a consent the host asks for with what it shows
//! (`grant_package_permissions`), not an action here.

use std::collections::BTreeMap;

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingValue, SettingsGroup};
use hanafubuki_agent_packages::PluginId;

use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection,
    SettingsRecord, ADD, DELETE, SAVE,
};
use crate::packages::AgentPackageStatus;
use crate::service::AgentApplicationService;
use crate::AgentPackageSource;

pub(super) const ID: &str = "packages";
const UPDATE: &str = "update";
const RETRY: &str = "retry";
const TRUST: &str = "trust";
const PURGE: &str = "purge";
const ENABLED: &str = "enabled";

fn fields() -> Vec<SettingSpec> {
    vec![SettingSpec {
        default: Some(SettingValue::Boolean(true)),
        ..SettingSpec::new(ENABLED, "Enabled", SettingKind::Boolean)
    }]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection =
        SettingsCollection::new(ID, "Installed plugins", SettingsGroup::Plugins, fields())
            .with_actions(vec![
                SettingsAction::collection(ADD, "Install a plugin")
                    .named("Plugin (PLUGIN@MARKETPLACE) or a directory"),
                SettingsAction::record(SAVE, "Save"),
                SettingsAction::record(UPDATE, "Update"),
                SettingsAction::record(RETRY, "Load again"),
                SettingsAction::record(TRUST, "Trust its hooks").confirmed(),
                SettingsAction::record(DELETE, "Uninstall, keep its data").confirmed(),
                SettingsAction::record(PURGE, "Uninstall and delete its data").confirmed(),
            ]);
    let packages = match service.packages().await {
        Ok(packages) => packages,
        Err(error) => return collection.unavailable(error),
    };
    SettingsCollection {
        records: packages.iter().map(record).collect(),
        ..collection
    }
}

fn record(package: &AgentPackageStatus) -> SettingsRecord {
    let ungranted = package
        .guest
        .as_ref()
        .and_then(|guest| guest.ungranted())
        .map(|_| "its code waits for its permissions to be granted".to_string());
    let untrusted = (!package.hooks_to_trust.is_empty()).then(|| {
        format!(
            "{} hook(s) wait to be trusted",
            package.hooks_to_trust.len()
        )
    });
    let guest_error = package
        .guest
        .as_ref()
        .and_then(|guest| guest.status.error.clone());
    let summary = match &package.description {
        Some(description) => format!("{} · {description}", package.version),
        None => package.version.clone(),
    };
    SettingsRecord {
        id: package.id.clone(),
        label: package.display_name.clone(),
        summary,
        state: Some(if package.enabled { "on" } else { "off" }.to_string()),
        issue: guest_error
            .or(ungranted)
            .or(untrusted)
            .or_else(|| package.issues.first().cloned()),
        read_only: None,
        values: BTreeMap::from([(ENABLED.to_string(), SettingValue::Boolean(package.enabled))]),
        secrets_set: Vec::new(),
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let id = request.record_id()?.to_string();
    let message = match request.action.as_str() {
        ADD => return install(service, &request, &id).await,
        SAVE => {
            for (key, value) in &request.values {
                let enabled = match (key.as_str(), value.as_bool()) {
                    (ENABLED, Some(enabled)) => enabled,
                    (ENABLED, None) => return Err("`enabled` is true or false".to_string()),
                    _ => return Err(format!("`{key}` is not a field of this record")),
                };
                service.set_package_enabled(&id, enabled).await?;
            }
            None
        }
        UPDATE => Some(waiting(&service.update_package(&id).await?.package)),
        RETRY => {
            let package = service.retry_package(&id).await?;
            Some(match package.issues.len() {
                0 => "loaded again".to_string(),
                count => format!("loaded again, with {count} issue(s)"),
            })
        }
        TRUST => {
            let package = service
                .package(&id)
                .await?
                .ok_or_else(|| format!("plugin `{id}` is not installed"))?;
            service
                .trust_hooks(package.hooks_to_trust)
                .await
                .map_err(|error| error.to_string())?;
            None
        }
        DELETE | PURGE => {
            let removed = service
                .uninstall_package(&id, request.action == PURGE)
                .await?;
            removed
                .purged_data
                .map(|_| "its data was deleted".to_string())
        }
        _ => return request.unknown_action(),
    };
    Ok(SettingsActionOutcome {
        record: None,
        message,
    })
}

/// Installs the package `source` names: an added marketplace's entry
/// (`PLUGIN@MARKETPLACE`), else a directory, filed under `local`.
async fn install(
    service: &AgentApplicationService,
    request: &SettingsActionRequest,
    source: &str,
) -> Result<SettingsActionOutcome, String> {
    if let Some((key, _)) = request.values.iter().find(|(key, _)| *key != ENABLED) {
        return Err(format!("`{key}` is not a field of this record"));
    }
    let source = match PluginId::parse(source) {
        Ok(id) => AgentPackageSource::Marketplace {
            marketplace: id.marketplace_name,
            plugin: id.plugin_name,
        },
        Err(_) => AgentPackageSource::Directory {
            path: source.into(),
            marketplace: None,
        },
    };
    let installed = service.install_package(source).await?;
    let id = installed.package.id.clone();
    if request
        .values
        .get(ENABLED)
        .and_then(|value| value.as_bool())
        == Some(false)
    {
        service.set_package_enabled(&id, false).await?;
    }
    Ok(SettingsActionOutcome {
        message: Some(waiting(&installed.package)),
        record: Some(id),
    })
}

/// What a package installed or updated waits for before all of it works.
fn waiting(package: &AgentPackageStatus) -> String {
    let mut parts = vec![format!("version {}", package.version)];
    if !package.hooks_to_trust.is_empty() {
        parts.push(format!(
            "{} hook(s) wait to be trusted",
            package.hooks_to_trust.len()
        ));
    }
    if package
        .guest
        .as_ref()
        .is_some_and(|guest| guest.ungranted().is_some())
    {
        parts.push("its code waits for its permissions to be granted".to_string());
    }
    parts.join("; ")
}
