//! What a guest may read from the Agent: the host functions `view` and
//! `invoke` reach only the two services below, each in a realm its guest's
//! entry shares with its host-services entry (`isolate` with the label
//! `guest:<id>`), which the guest's entry declares (`inject`).
//!
//! Everything else stays out of reach: kardia's checked access finds only
//! what the guest's chain of fibers provides or declares, and nothing else
//! on that chain has a JSON view (`view`) or is a guest's service
//! (`invoke`). A guest that provides a service other than its `plugin` is
//! stopped (`Shared::on_service`).

use std::sync::Arc;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::WorkspaceAccess;
use kardia_core::{Context, PluginDef};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::Value;

use crate::package::{access_name, GuestPackage};
use crate::spawn::{SpawnContext, WorkspaceRoots};

/// The Agent's workspace as the guest may reach it: `{ "access", "roots" }`,
/// the roots only when it may read them.
pub(crate) const WORKSPACE_SERVICE: &str = "workspace";
/// The guest's own settings, its configuration.
pub(crate) const SETTINGS_SERVICE: &str = "settings";

/// The plugin of `package`'s host-services entry: it provides the workspace
/// as granted, read again at every view, and the settings it is configured
/// with.
pub(crate) fn host_services(package: &GuestPackage, context: &Arc<SpawnContext>) -> PluginDef {
    let granted = package.granted.clone().unwrap_or_default();
    let access = package.permissions.workspace.min(granted.workspace);
    let roots = context.workspace.clone();
    PluginDef::new(move |ctx: Context, settings: Value| {
        let roots = roots.clone();
        async move {
            ctx.provide_serialized_as(WORKSPACE_SERVICE, WorkspaceView { access, roots })?;
            ctx.provide_serialized_as(SETTINGS_SERVICE, settings)?;
            Ok(())
        }
    })
    .name(format!("guest-host:{}", package.id.key()))
    .provides([WORKSPACE_SERVICE, SETTINGS_SERVICE])
}

struct WorkspaceView {
    access: WorkspaceAccess,
    roots: WorkspaceRoots,
}

impl Serialize for WorkspaceView {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let roots: Vec<String> = match self.access {
            WorkspaceAccess::None => Vec::new(),
            WorkspaceAccess::Read | WorkspaceAccess::Write => (self.roots)()
                .iter()
                .filter_map(|root| ResolvedPath::resolve(root).ok())
                .map(|root| root.native_text().to_string())
                .collect(),
        };
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("access", access_name(self.access))?;
        map.serialize_entry("roots", &roots)?;
        map.end()
    }
}
