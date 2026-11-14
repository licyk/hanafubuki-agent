//! The turn's Skill catalog: built from the Skill service when a turn is
//! admitted, narrowed to the Skills whose MCP dependencies are reachable (the
//! others listed as unavailable, with why), and frozen on the turn for the
//! Skill tools, the catalog listing and the kernel's Skill selection, with
//! the names a plain `$mention` names instead of a Skill.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    BlockingService, RegistryService, SelectableSkill, SelectableSkills, ToolRegistry,
    TurnAdmissionContributor, TurnAdmissionFuture, TurnAdmissionRequest, TurnExtensions,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_mcp::McpToolLease;
use hanafubuki_agent_skills::{SkillTurnCatalog, SkillTurnEntry, SkillTurnUnavailable};

use crate::service::AgentMcpApplication;
use crate::skills::AgentSkillApplication;
use crate::SkillAvailabilityState;

/// Freezes the turn's [`SkillTurnCatalog`] and the [`SelectableSkills`] the
/// kernel resolves references and `$mention`s against. `tools` is the
/// Agent's tool registry, whose names a `$mention` names instead of a Skill.
pub(super) struct SkillTurnAdmission {
    pub(super) skills: AgentSkillApplication,
    pub(super) mcp: AgentMcpApplication,
    pub(super) tools: Arc<ToolRegistry>,
    pub(super) blocking: Arc<BlockingService>,
}

impl TurnAdmissionContributor for SkillTurnAdmission {
    fn id(&self) -> &str {
        "skills"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute<'a>(
        &'a self,
        request: &'a TurnAdmissionRequest,
        _cancellation: CancellationToken,
    ) -> TurnAdmissionFuture<'a> {
        let skills = self.skills.clone();
        let workspace = request.workspace.clone();
        let mcp = self.mcp.clone();
        Box::pin(async move {
            let catalog = self
                .blocking
                .run("agent turn Skill catalog worker", move || {
                    let mut catalog = match workspace {
                        Some(workspace) => skills.turn_catalog_for_roots(&workspace.roots)?,
                        None => skills.turn_catalog()?,
                    };
                    retain_available_skill_tools(&mut catalog, &mcp);
                    Ok::<_, String>(catalog)
                })
                .await
                .map_err(|error| error.to_string())??;
            let tool_names = self
                .tools
                .registry()
                .snapshot()
                .items()
                .map(|tool| tool.descriptor().name.qualified())
                .collect();
            let exclusions = mention_exclusions(tool_names, &self.mcp.mcp.tool_leases());
            Ok(turn_extensions(catalog, exclusions))
        })
    }
}

/// The names a plain `$mention` names instead of a Skill: the registered
/// tools and, for every MCP tool that is not hidden, its server, its name
/// and its flattened alias. Sorted and deduplicated ASCII case-insensitively.
/// The kernel's own tools are the kernel's to exclude.
pub(super) fn mention_exclusions(mut names: Vec<String>, leases: &[McpToolLease]) -> Vec<String> {
    for lease in leases
        .iter()
        .filter(|lease| lease.binding.exposure != hanafubuki_agent_mcp::McpExposurePolicy::Hidden)
    {
        names.push(lease.binding.server_id.as_str().to_string());
        names.push(lease.binding.specification.name.local_name().to_string());
        names.push(lease.binding.specification.name.flattened_alias());
    }
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    names
}

/// The catalog for the Skill tools and the listing, and every entry of it as
/// a Skill the turn may select (an explicit-only Skill too), unless a plain
/// `$mention` of it names one of `mention_exclusions`.
pub(super) fn turn_extensions(
    catalog: SkillTurnCatalog,
    mention_exclusions: Vec<String>,
) -> TurnExtensions {
    let selectable = SelectableSkills {
        mention_exclusions,
        skills: catalog
            .entries
            .iter()
            .map(|entry| SelectableSkill {
                id: entry.id.as_str().to_string(),
                name: entry.name.clone(),
                context: selected_skill_context(entry),
            })
            .collect(),
    };
    TurnExtensions::default()
        .with(Arc::new(catalog))
        .with(Arc::new(selectable))
}

/// The instructions a selected or adopted Skill adds to the conversation:
/// the one delivery of its `SKILL.md`, with the id its other files are read
/// by and the directory its scripts run in.
pub(super) fn selected_skill_context(entry: &SkillTurnEntry) -> String {
    format!(
        "<skill>\n<name>{}</name>\n<skill_id>{}</skill_id>\n<directory>{}</directory>\n{}\n</skill>",
        entry.name, entry.id, entry.directory, entry.document
    )
}

/// Moves every Skill whose declared MCP servers are not all Ready out of the
/// usable entries and into `unavailable`, naming the first server that is
/// not and its state. Installing or connecting it stays the user's action.
fn retain_available_skill_tools(snapshot: &mut SkillTurnCatalog, mcp: &AgentMcpApplication) {
    let mut unavailable = Vec::new();
    snapshot.entries.retain(|skill| {
        let reason = match mcp.skill_dependency_statuses(&skill.dependencies) {
            Ok(dependencies) => {
                let Some(status) = dependencies
                    .iter()
                    .find(|status| status.state != SkillAvailabilityState::Ready)
                else {
                    return true;
                };
                format!(
                    "MCP server `{}` is {}: {}",
                    status.dependency.server_id,
                    serde_json::to_value(status.state)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned))
                        .unwrap_or_else(|| "unavailable".to_string()),
                    status
                        .reason
                        .as_deref()
                        .unwrap_or("dependency action is required")
                )
            }
            Err(error) => format!("its MCP dependencies could not be checked: {error}"),
        };
        log::debug!(
            "agent Skill unavailable in turn catalog skill={}: {reason}",
            skill.id
        );
        unavailable.push(SkillTurnUnavailable {
            id: skill.id.clone(),
            name: skill.name.clone(),
            reason,
        });
        false
    });
    snapshot.unavailable = unavailable;
}

#[cfg(test)]
mod tests;
