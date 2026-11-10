//! The installed plugins' code: the guest host the `plugins` row mounts,
//! the guest each code package gets, and the settings the guest host's
//! tree is written to.

use std::sync::Arc;

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_foundation::blocking::{run_blocking_operation, BlockingOperationPort};
use hanafubuki_agent_host::RestartPolicy;
use hanafubuki_agent_packages::{InstalledPackage, PluginId};
use hanafubuki_agent_plugin_packages::{
    GuestHost, GuestHostInputs, GuestPackage, GuestSettingsPort,
};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::Sandbox;

use crate::WorkspaceRootsPort;

/// The guest host over the instance's executor: every guest confined by
/// the platform's sandbox (the restricted token on Windows whatever
/// `[windows] sandbox` says, which governs the model's commands), reading
/// or writing the workspace roots as granted.
pub(crate) fn guest_host(
    executor: &ExecutorManager,
    config: &AgentConfig,
    blocking: Arc<dyn BlockingOperationPort>,
    workspace: Arc<dyn WorkspaceRootsPort>,
    packages: Vec<GuestPackage>,
) -> GuestHost {
    GuestHost::new(
        GuestHostInputs {
            executor: executor.clone(),
            sandbox: Sandbox::detect(),
            settings: Arc::new(PackageSettingsWriter {
                config: config.clone(),
                blocking,
                writes: Arc::default(),
            }),
            workspace: Arc::new(move || {
                workspace
                    .roots()
                    .map(|roots| roots.roots().to_vec())
                    .unwrap_or_default()
            }),
            restart_policy: RestartPolicy::default(),
            call_timeout: hanafubuki_agent_kernel::FIRST_BATCH_HARD_LIMITS.plugin_call_deadline,
        },
        packages,
    )
}

/// `package`'s guest, when its manifest is this project's and has a
/// runtime.
pub(super) fn guest_package(package: &InstalledPackage, enabled: bool) -> Option<GuestPackage> {
    let manifest = package.manifest.as_ref()?;
    let code = manifest.manifest.code.as_ref()?;
    let record = package.record.as_ref();
    Some(GuestPackage {
        id: package.id.clone(),
        version: package.version.clone(),
        root: package.root.clone(),
        data_root: package.data_root.clone(),
        name: manifest.manifest.name.clone(),
        runtime: code.runtime.clone()?,
        permissions: code.permissions.clone(),
        granted: record.and_then(|record| record.granted_permissions.clone()),
        content_hash: record.and_then(|record| record.content_hash.clone()),
        enabled,
    })
}

/// Writes what the guest host's tree changes on its own to `[plugins]`, one
/// write at a time on the blocking pool, and only a value that differs
/// from the settings'.
struct PackageSettingsWriter {
    config: AgentConfig,
    blocking: Arc<dyn BlockingOperationPort>,
    writes: Arc<tokio::sync::Mutex<()>>,
}

impl GuestSettingsPort for PackageSettingsWriter {
    fn set_enabled(&self, id: &str, enabled: bool) {
        let Ok(id) = PluginId::parse(id) else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            log::warn!("plugin {id}: whether it is on was not written: no runtime");
            return;
        };
        let (config, blocking, writes) = (
            self.config.clone(),
            self.blocking.clone(),
            self.writes.clone(),
        );
        runtime.spawn(async move {
            let _write = writes.lock().await;
            let label = "agent plugin package settings";
            let written = run_blocking_operation(blocking.as_ref(), label, move || {
                write_enabled(&config, &id, enabled)
            })
            .await
            .and_then(|written| written);
            if let Err(error) = written {
                log::warn!("a plugin's setting was not written: {error}");
            }
        });
    }
}

fn write_enabled(config: &AgentConfig, id: &PluginId, enabled: bool) -> Result<(), String> {
    let settings = config.settings()?;
    if settings.package_enabled(id) == enabled {
        return Ok(());
    }
    let mut entry = settings
        .plugins
        .packages
        .get(&id.key())
        .cloned()
        .unwrap_or_default();
    entry.enabled = Some(enabled);
    config.set_package_settings(id, Some(&entry))?;
    log::info!("plugin {id} turned {}", if enabled { "on" } else { "off" });
    Ok(())
}
