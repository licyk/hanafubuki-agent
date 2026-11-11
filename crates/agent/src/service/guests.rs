//! The installed plugins' code: the user's grant of what it may reach and
//! its settings (`retry_package` also starts a guest that failed again).

use hanafubuki_agent_foundation::blocking::run_blocking_operation;
use hanafubuki_agent_packages::{PackagePermissions, PluginId};
use serde_json::Value;

use super::AgentApplicationService;

impl AgentApplicationService {
    /// Records that the user granted the code of the installed package `id`
    /// (`<plugin>@<marketplace>`) `permissions`, those its consent showed,
    /// then reloads the packages: a guest that waited for them starts, if
    /// its package is on. A guest starts only when what its manifest asks
    /// for is granted; an update that asks for more waits for a new grant.
    pub async fn grant_package_permissions(
        &self,
        id: &str,
        permissions: PackagePermissions,
    ) -> Result<(), String> {
        let id = PluginId::parse(id).map_err(|error| error.to_string())?;
        let packages = self.packages.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent plugin package grant",
            move || packages.grant(&id, permissions),
        )
        .await??;
        self.reload_packages().await
    }

    /// Replaces the settings of the code of the installed package `id`
    /// (`[plugins."<id>"].settings`), or removes them (`Value::Null`), then
    /// reloads the packages: its guest starts again with them, and views
    /// them as its `settings`. They are an object, which must fit what the
    /// package's manifest says they are (`configSchema`) when it says: the
    /// JSON Schema, and each value the bounds of the option it lowers to,
    /// so the guest receives them as written.
    pub async fn set_package_settings(&self, id: &str, settings: Value) -> Result<(), String> {
        let id = PluginId::parse(id).map_err(|error| error.to_string())?;
        let settings = match settings {
            Value::Null => None,
            Value::Object(settings) => Some(settings),
            _ => return Err("a plugin's settings are an object".to_string()),
        };
        let loaded = self.packages.loaded();
        let package = loaded
            .packages
            .iter()
            .find(|package| package.id == id)
            .ok_or_else(|| format!("plugin `{id}` is not installed"))?;
        if package.guest.is_none() {
            return Err(format!("plugin `{id}` has no code to configure"));
        }
        if let (Some(schema), Some(settings)) = (&package.config_schema, &settings) {
            hanafubuki_agent_kernel::validate_dynamic_tool_arguments(
                &Value::Object(schema.clone()),
                &Value::Object(settings.clone()),
            )
            .map_err(|error| {
                let reason = match error {
                    hanafubuki_agent_kernel::AgentToolCatalogError::InvalidArguments(reason) => {
                        reason
                    }
                    other => other.to_string(),
                };
                format!("the settings do not fit its configSchema: {reason}")
            })?;
        }
        if let (Some(options), Some(settings)) = (&package.options, &settings) {
            for option in &options.schema.options {
                if let Some(value) = settings.get(&option.key) {
                    option.check(value).map_err(|reason| {
                        format!("the settings do not fit its configSchema: {reason}")
                    })?;
                }
            }
        }
        let config = self.config.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent plugin package settings",
            move || {
                config
                    .set_package_code_settings(&id, settings.as_ref())
                    .map(drop)
            },
        )
        .await??;
        self.reload_packages().await
    }
}
