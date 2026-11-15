//! Layer forms include role overrides and registered package fields.
use super::profiles::quoted;
use crate::service::AgentApplicationService;
use hanafubuki_agent_config::{patch_setting_values, SettingKind, SettingSpec, SettingsLayer};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub(super) async fn extend(service: &AgentApplicationService, fields: &mut Vec<SettingSpec>) {
    let mut roles = service
        .roles()
        .roles()
        .map(|role| role.name.clone())
        .collect::<BTreeSet<_>>();
    roles.extend(service.roles().unusable().map(|role| role.name.clone()));
    if let Ok(document) = service.config.user_document() {
        roles.extend(
            document
                .get("agents")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|roles| roles.keys().cloned()),
        );
        for profile in document
            .get("profiles")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|profiles| profiles.values())
        {
            roles.extend(
                profile
                    .get("agents")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|roles| roles.keys().cloned()),
            );
        }
    }
    for name in roles
        .into_iter()
        .filter(|name| hanafubuki_agent_config::is_role_name(name))
    {
        fields.extend(super::roles::fields().into_iter().map(|field| SettingSpec {
            key: format!("agents.{}.{}", quoted(&name), field.key),
            label: format!("Role {name}: {}", field.label),
            default: None,
            ..field
        }));
    }
    if let Ok(packages) = service.packages().await {
        for package in &packages {
            fields.push(SettingSpec::new(
                format!("plugins.{}.enabled", quoted(&package.id)),
                format!("Plugin {} enabled", package.id),
                SettingKind::Boolean,
            ));
        }
        fields.extend(
            super::package_settings::sections(&packages)
                .into_iter()
                .flat_map(|section| section.settings)
                .map(|field| SettingSpec {
                    default: None,
                    ..field
                }),
        );
    }
}

pub(super) async fn write(
    service: &AgentApplicationService,
    layer: SettingsLayer,
    values: Map<String, Value>,
    fields: Vec<SettingSpec>,
) -> Result<String, String> {
    let (_, mut document) = service.config.layer_document(&layer)?;
    patch_setting_values(&mut document, &values.clone().into_iter().collect())?;
    let loaded = service.packages.loaded();
    for package in &loaded.packages {
        let prefix = format!("plugins.{}.settings.", quoted(&package.id.key()));
        if !values.keys().any(|key| key.starts_with(&prefix)) {
            continue;
        }
        if let (Some(schema), Some(settings)) = (
            &package.config_schema,
            document
                .get("plugins")
                .and_then(|plugins| plugins.get(package.id.key()))
                .and_then(|plugin| plugin.get("settings")),
        ) {
            hanafubuki_agent_kernel::validate_dynamic_tool_arguments(
                &Value::Object(schema.clone()),
                settings,
            )
            .map_err(|error| format!("Plugin {} settings: {error}", package.id))?;
        }
    }
    let config = service.config.clone();
    let packages_changed = values.keys().any(|key| key.starts_with("plugins."));
    let label = format!("{layer:?}");
    let target = layer.clone();
    let written = values.keys().cloned().collect::<Vec<_>>();
    let path = hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "write selected configuration layer",
        move || config.write_layer_fields(&layer, &values, &fields),
    )
    .await??;
    service.apply_settings_to_plugins();
    if packages_changed {
        service.reload_packages().await?;
    }
    service
        .roles
        .rebuild(&service.config, || service.packages.loaded().roles());
    let sources = service.config.setting_sources()?;
    let shadowed = written
        .iter()
        .filter_map(|key| {
            let source = sources.get(key)?;
            use hanafubuki_agent_config::SettingSource;
            let shadows = matches!(
                (&target, source),
                (
                    SettingsLayer::User,
                    SettingSource::Profile { .. }
                        | SettingSource::Project { .. }
                        | SettingSource::Override,
                ) | (
                    SettingsLayer::Profile(_),
                    SettingSource::Project { .. } | SettingSource::Override,
                ) | (SettingsLayer::Project, SettingSource::Override)
            );
            shadows.then(|| format!("{key}: {source:?}"))
        })
        .collect::<Vec<_>>();
    let mut message = format!(
        "Saved {label} to {}; precedence: user < selected profile < trusted project < command line",
        path.display()
    );
    if !shadowed.is_empty() {
        message.push_str(&format!("\nShadowed: {}", shadowed.join(", ")));
    }
    if let SettingsLayer::Profile(name) = target {
        let selected = service.config.layer_sources()?.iter().any(|source| matches!(source, hanafubuki_agent_config::ConfigLayerSource::Profile { name: active } if *active == name));
        if !selected {
            message.push_str("\nThis profile is not selected for the current Agent");
        }
    }
    Ok(message)
}
