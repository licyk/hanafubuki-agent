//! An installed package as the facade reports it.

use hanafubuki_agent_hooks::{HookEntry, HookSource, HookTrust};
use hanafubuki_agent_packages::{InstallRecord, ManifestFormat, PackageSource};
use hanafubuki_agent_skills::SkillCatalogEntry;
use serde::Serialize;

use super::LoadedPackage;

/// One installed package: whether it is on, what it contributes and what
/// is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
    /// source); `None` when the install record has no entry for it.
    pub source: Option<PackageSource>,
    pub enabled: bool,
    /// What it contributes while it is on.
    pub content: AgentPackageContent,
    /// The keys of its hooks that do not run until they are trusted (new,
    /// or changed since, or in a package whose contents changed since).
    pub hooks_to_trust: Vec<String>,
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
}

/// `package` with its install `record`, its Skills among `skills` and its
/// hooks among `hooks`.
pub(super) fn package_status(
    package: &LoadedPackage,
    record: Option<&InstallRecord>,
    skills: &[SkillCatalogEntry],
    hooks: &[HookEntry],
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
        issues: package.issues.clone(),
        id,
    }
}
