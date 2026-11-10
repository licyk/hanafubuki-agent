//! A code package as the guest host runs it, and its loader entry.

use std::path::PathBuf;

use hanafubuki_agent_packages::{
    PackageContributes, PackagePermissions, PackageRuntime, PluginId, RuntimeKind, WorkspaceAccess,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::host_services::{SETTINGS_SERVICE, WORKSPACE_SERVICE};

/// The service every guest provides, in a realm of its own: the Agent calls
/// a guest through it. A guest declares it in its metadata
/// (`Meta::plugin(<manifest name>).provide("plugin")`) and provides it in
/// `apply`.
pub const GUEST_SERVICE: &str = "plugin";

/// What ends the id of a guest's host-services entry: the loader entry,
/// beside the guest's, whose plugin provides what the guest may view.
pub(crate) const HOST_ENTRY_SUFFIX: &str = "#host";

/// An installed package with code, as the facade read it from the store and
/// the settings.
#[derive(Clone, Debug, PartialEq)]
pub struct GuestPackage {
    pub id: PluginId,
    pub version: String,
    /// The installed package (`PLUGIN_ROOT`), which its guest only reads.
    pub root: PathBuf,
    /// Its own data directory (`PLUGIN_DATA`), the guest's working
    /// directory and the one place it writes.
    pub data_root: PathBuf,
    /// The manifest's name, which the guest declares as its plugin's.
    pub name: String,
    pub runtime: PackageRuntime,
    /// What its manifest asks for.
    pub permissions: PackagePermissions,
    /// What the user granted, from the install record.
    pub granted: Option<PackagePermissions>,
    /// The content hash the install recorded, which the package must still
    /// have when its guest starts.
    pub content_hash: Option<String>,
    /// What its manifest declares the guest contributes.
    pub contributes: PackageContributes,
    /// Its settings (`[plugins."<id>"].settings`): the guest's configuration
    /// and its `settings` view.
    pub settings: Map<String, Value>,
    /// Whether the settings leave it on.
    pub enabled: bool,
}

impl GuestPackage {
    /// Why its guest may not start at all: a runtime this host does not
    /// run, or a permission the user has not granted.
    pub(crate) fn refusal(&self) -> Option<String> {
        if self.runtime.kind != RuntimeKind::Process {
            return Some("its runtime is not supported yet: only \"process\" runs".to_string());
        }
        let Some(granted) = &self.granted else {
            return Some("its permissions are not granted".to_string());
        };
        let mut missing = Vec::new();
        if self.permissions.network && !granted.network {
            missing.push("network".to_string());
        }
        if self.permissions.workspace > granted.workspace {
            missing.push(format!(
                "workspace {}",
                access_name(self.permissions.workspace)
            ));
        }
        missing.extend(
            self.permissions
                .env
                .iter()
                .filter(|name| !granted.env.contains(name))
                .map(|name| format!("env {name}")),
        );
        (!missing.is_empty()).then(|| format!("permissions not granted: {}", missing.join(", ")))
    }

    /// Whether its guest runs: it is on and may start.
    pub(crate) fn runs(&self) -> bool {
        self.enabled && self.refusal().is_none()
    }

    /// Its loader entries, off unless the guest runs: the guest's, and its
    /// host services' beside it. The guest's `plugin` is a realm of its own;
    /// what it may view (`workspace`, `settings`) is in a realm the two
    /// entries share, which the guest's entry declares (`inject`), so the
    /// guest starts once its host services run. The plugins are named by
    /// everything their start depends on, so a new version, a changed
    /// install or grant starts them anew; the settings are their
    /// configuration, so a change restarts both.
    pub(crate) fn entries(&self) -> [Value; 2] {
        let label = format!("guest:{}", self.id.key());
        let config = Value::Object(self.settings.clone());
        [
            json!({
                "id": self.host_entry_id(),
                "name": self.host_entry_name(),
                "disabled": !self.runs(),
                "isolate": { WORKSPACE_SERVICE: label, SETTINGS_SERVICE: label },
                "config": config,
            }),
            json!({
                "id": self.id.key(),
                "name": self.entry_name(),
                "disabled": !self.runs(),
                "isolate": {
                    GUEST_SERVICE: true,
                    WORKSPACE_SERVICE: label,
                    SETTINGS_SERVICE: label,
                },
                "inject": [WORKSPACE_SERVICE, SETTINGS_SERVICE],
                "config": config,
            }),
        ]
    }

    pub(crate) fn host_entry_id(&self) -> String {
        format!("{}{HOST_ENTRY_SUFFIX}", self.id.key())
    }

    pub(crate) fn host_entry_name(&self) -> String {
        format!("{}{HOST_ENTRY_SUFFIX}", self.entry_name())
    }

    pub(crate) fn entry_name(&self) -> String {
        let identity = format!(
            "{:?}",
            (
                &self.version,
                &self.content_hash,
                &self.runtime,
                &self.granted,
                &self.permissions,
                &self.root,
                &self.data_root,
                &self.name,
            )
        );
        let digest = hex::encode(Sha256::digest(identity.as_bytes()));
        format!("{}#{}", self.id.key(), &digest[..16])
    }
}

pub(crate) fn access_name(access: WorkspaceAccess) -> &'static str {
    match access {
        WorkspaceAccess::None => "none",
        WorkspaceAccess::Read => "read",
        WorkspaceAccess::Write => "write",
    }
}
