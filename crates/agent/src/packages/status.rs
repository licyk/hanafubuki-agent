//! An installed package as the facade reports it.

use hanafubuki_agent_hooks::{HookEntry, HookSource};
use hanafubuki_agent_packages::ManifestFormat;
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
    pub enabled: bool,
    /// What it contributes while it is on.
    pub content: AgentPackageContent,
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

/// `package` with its Skills among `skills` and its hooks among `hooks`.
pub(super) fn package_status(
    package: &LoadedPackage,
    skills: &[SkillCatalogEntry],
    hooks: &[HookEntry],
) -> AgentPackageStatus {
    let id = package.id.key();
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
        hooks: hooks
            .iter()
            .filter(
                |hook| matches!(&hook.source, HookSource::Plugin { id: owner, .. } if *owner == id),
            )
            .map(|hook| hook.key.clone())
            .collect(),
    };
    AgentPackageStatus {
        version: package.version.clone(),
        format: package.format,
        display_name: package.display_name.clone(),
        description: package.description.clone(),
        enabled: package.enabled,
        content,
        issues: package.issues.clone(),
        id,
    }
}
