//! An installed package as the facade reports it.

use hanafubuki_agent_hooks::{HookEntry, HookSource, HookTrust};
use hanafubuki_agent_packages::{
    InstallRecord, ManifestFormat, PackagePermissions, PackageSource, RuntimeKind,
};
use hanafubuki_agent_plugin_packages::GuestStatus;
use hanafubuki_agent_skills::SkillCatalogEntry;
use serde::Serialize;
use serde_json::{Map, Value};

use super::LoadedPackage;
use crate::host::AgentPluginOptionsStatus;

/// One installed package: whether it is on, what it contributes and what
/// is wrong with it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageStatus {
    /// `<plugin>@<marketplace>`.
    pub id: String,
    pub version: String,
    /// Its manifest's format; `None` when the manifest does not read.
    pub format: Option<ManifestFormat>,
    pub display_name: String,
    pub description: Option<String>,
    /// Where it was installed from (a directory, or a marketplace entry's
    /// source), or the directory it is linked to; `None` when the install
    /// record has no entry for it.
    pub source: Option<PackageSource>,
    pub enabled: bool,
    /// What it contributes while it is on.
    pub content: AgentPackageContent,
    /// The keys of its hooks that do not run until they are trusted (new,
    /// or changed since, or in a package whose contents changed since).
    pub hooks_to_trust: Vec<String>,
    /// Its code, for a package in this project's format with a runtime.
    pub guest: Option<AgentPackageGuest>,
    /// What could not be read or used: its manifest's fields, its
    /// components, an MCP server that gives way to another of its name.
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageContent {
    /// The ids of its Skills (`plugin:<id>:<path>`).
    pub skills: Vec<String>,
    /// The names of its MCP servers that run.
    pub mcp_servers: Vec<String>,
    /// The keys of its hooks.
    pub hooks: Vec<String>,
    /// The names of its roles (`<plugin>:<role>`), usable or not.
    pub roles: Vec<String>,
}

/// A code package's guest: what it may reach and how it runs.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageGuest {
    /// What runs its code: a process, or a WebAssembly module, which
    /// reaches nothing outside itself and needs no grant.
    pub runtime: RuntimeKind,
    /// What its manifest asks for, which a process guest needs granted to
    /// start (nothing, for a WASM guest).
    pub permissions: PackagePermissions,
    /// What the user granted; none before a grant.
    pub granted_permissions: Option<PackagePermissions>,
    /// Its state, why it is not running, how its last process ended and
    /// the end of its stderr.
    pub status: GuestStatus,
    /// Its settings (`[plugins."<id>"].settings`) as its guest receives
    /// them: checked against its `options`, so a value that did not fit is
    /// its default here, or left out.
    pub settings: Map<String, Value>,
    /// What its manifest says its settings are (`configSchema`, a JSON
    /// Schema), which a change of them is checked against.
    pub config_schema: Option<Map<String, Value>>,
    /// The options its `configSchema` lowers to, their values and what
    /// the settings set that could not be used; `None` without a schema.
    pub options: Option<AgentPluginOptionsStatus>,
}

impl AgentPackageGuest {
    /// What it asks for and was not granted, which a user's consent grants:
    /// none for a WASM guest, which needs no grant.
    pub fn ungranted(&self) -> Option<&PackagePermissions> {
        let granted = self.granted_permissions.as_ref() == Some(&self.permissions);
        (self.runtime == RuntimeKind::Process && !granted).then_some(&self.permissions)
    }
}

/// `package` with its install `record`, its Skills among `skills`, its
/// hooks among `hooks` and its `guest`.
pub(super) fn package_status(
    package: &LoadedPackage,
    record: Option<&InstallRecord>,
    skills: &[SkillCatalogEntry],
    hooks: &[HookEntry],
    guest: Option<AgentPackageGuest>,
) -> AgentPackageStatus {
    let id = package.id.key();
    let own_hooks = hooks
        .iter()
        .filter(|hook| matches!(&hook.source, HookSource::Plugin { id: owner, .. } if *owner == id))
        .collect::<Vec<_>>();
    let content = AgentPackageContent {
        skills: skills
            .iter()
            .filter(|skill| skill.id.plugin() == Some(id.as_str()))
            .map(|skill| skill.id.as_str().to_string())
            .collect(),
        mcp_servers: package
            .mcp_servers
            .iter()
            .filter(|server| server.enabled)
            .map(|server| server.id.as_str().to_string())
            .collect(),
        hooks: own_hooks.iter().map(|hook| hook.key.clone()).collect(),
        roles: package.roles.iter().map(|role| role.name.clone()).collect(),
    };
    let hooks_to_trust = own_hooks
        .iter()
        .filter(|hook| hook.trust != HookTrust::Trusted)
        .map(|hook| hook.key.clone())
        .collect();
    AgentPackageStatus {
        version: package.version.clone(),
        format: package.format,
        display_name: package.display_name.clone(),
        description: package.description.clone(),
        source: record.and_then(|record| record.source.clone()),
        enabled: package.enabled,
        content,
        hooks_to_trust,
        guest,
        issues: package.issues.clone(),
        id,
    }
}
