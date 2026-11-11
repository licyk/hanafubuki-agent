//! The plugins the Agent runs, each with its configuration derived from the
//! settings, and the host's rows among them. Adding a capability plugin adds
//! one row here; a row derived from the settings follows them while the
//! Agent runs, and any row can be turned off through `disabledPlugins`.

use hanafubuki_agent_config::AgentWebSearchProvider;
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_hooks::HooksEngine;
use hanafubuki_agent_mcp::McpManager;
use hanafubuki_agent_plugin_execution::execution_plugin;
use hanafubuki_agent_plugin_hooks::{hooks_plugin, WorkingDirectory};
use hanafubuki_agent_plugin_mcp::mcp_plugin;
use hanafubuki_agent_plugin_web::{
    web_plugin, WebPluginConfig, WebProviderSetting, WebProxySetting,
};
use hanafubuki_agent_plugin_workspace_fs::workspace_fs_plugin;
use hanafubuki_agent_plugin_worktree::worktree_plugin;
use hanafubuki_agent_workspace_fs::WorkspaceFsService;
use hanafubuki_agent_worktree::{GitQueries, WorktreeManager};
use std::path::PathBuf;
use std::sync::Arc;

use super::{AgentPluginDescriptor, AgentPluginGroup, AgentPluginRow, AgentPluginSettings};
use crate::conversation_reader::AgentConversationReader;
use crate::goals::AgentGoalApplication;
use crate::packages::PackageReload;
use crate::plugins::{
    goal_plugin, memory_plugin, packages_plugin, permission_mode_plugin, recap_plugin,
    review_plugin, skills_plugin, subagents_plugin, titles_plugin, view_image_plugin,
    workspace_context_plugin, SUBAGENTS_PLUGIN,
};
use crate::roles::AgentRoleCatalogReader;
use crate::service::AgentMcpApplication;
use crate::session_driver::AgentSessionDriver;
use crate::skills::AgentSkillApplication;
use crate::titles::AgentTitleApplication;
use crate::{AgentMemoryApplication, AgentProxy, WorkspaceRootsPort};

/// Services the application constructs once and hands to the plugins that
/// adapt them; they are not serializable configuration.
pub(crate) struct AgentPluginInputs {
    /// Where the user's own `AGENTS.md` is read from.
    pub(crate) user_instructions_dir: PathBuf,
    /// The application the environment section names.
    pub(crate) application: String,
    pub(crate) workspace: Arc<dyn WorkspaceRootsPort>,
    pub(crate) files: WorkspaceFsService,
    pub(crate) executions: AgentExecutionService,
    pub(crate) mcp: Arc<McpManager>,
    pub(crate) memory: AgentMemoryApplication,
    pub(crate) skills: AgentSkillApplication,
    /// Narrows the turn's Skill catalog to reachable MCP dependencies.
    pub(crate) mcp_application: AgentMcpApplication,
    pub(crate) titles: AgentTitleApplication,
    /// What a recap reads of a session.
    pub(crate) conversations: AgentConversationReader,
    /// The sessions' goals.
    pub(crate) goals: AgentGoalApplication,
    /// How the subagents are created and run.
    pub(crate) driver: AgentSessionDriver,
    /// The roles a subagent is spawned as.
    pub(crate) roles: AgentRoleCatalogReader,
    /// The worktrees, under the data directory, over the instance's
    /// executor.
    pub(crate) worktrees: WorktreeManager,
    /// What a review is chosen from, over the instance's executor.
    pub(crate) git: GitQueries,
    /// The user's and a trusted project's hooks, over the instance's
    /// executor.
    pub(crate) hooks: Arc<HooksEngine>,
    /// Where a hook's command runs: the first workspace root.
    pub(crate) working_directory: WorkingDirectory,
    /// The installed plugins, which the `plugins` row summarizes, and their
    /// reload, which it runs once a linked package changed.
    pub(crate) packages: PackageReload,
}

/// The rows the host mounts after its core ones, and the descriptors of the
/// host's rows among them (the Agent's own are `agent_plugin_descriptors`).
pub(crate) struct AgentPluginManifest {
    pub(crate) rows: Vec<AgentPluginRow>,
    pub(crate) host_descriptors: Vec<AgentPluginDescriptor>,
}

/// The manifest rows in mount order. Each declares its group, which the
/// plugin manager groups them by.
///
/// The host's rows (`host`) mount after the Agent's capability plugins and
/// before its context plugins. Mount order shows nowhere in a request: each
/// renderer's `order` places its section within its slot.
pub(crate) fn agent_plugin_manifest(
    inputs: AgentPluginInputs,
    host: Vec<(AgentPluginRow, AgentPluginDescriptor)>,
) -> AgentPluginManifest {
    use AgentPluginGroup::{Capability, Context};
    let mut rows = vec![
        AgentPluginRow::from_settings(Capability, web_plugin(), |settings| {
            serde_json::to_value(web_config(settings)?).map_err(|error| error.to_string())
        }),
        AgentPluginRow::unconfigured(Capability, workspace_fs_plugin(inputs.files.clone())),
        AgentPluginRow::unconfigured(Capability, view_image_plugin(inputs.files)),
        AgentPluginRow::unconfigured(Capability, execution_plugin(inputs.executions)),
        AgentPluginRow::unconfigured(Capability, mcp_plugin(inputs.mcp)),
        AgentPluginRow::unconfigured(Capability, memory_plugin(inputs.memory)),
        AgentPluginRow::unconfigured(
            Capability,
            skills_plugin(inputs.skills, inputs.mcp_application),
        ),
        AgentPluginRow::unconfigured(Capability, titles_plugin(inputs.titles)),
        AgentPluginRow::unconfigured(Capability, recap_plugin(inputs.conversations)),
        AgentPluginRow::unconfigured(Capability, goal_plugin(inputs.goals)),
        AgentPluginRow::from_settings(
            Capability,
            subagents_plugin(inputs.driver.clone(), inputs.roles),
            |settings| settings.options_value(SUBAGENTS_PLUGIN),
        ),
        AgentPluginRow::unconfigured(Capability, review_plugin(inputs.driver, inputs.git)),
        AgentPluginRow::unconfigured(Capability, worktree_plugin(inputs.worktrees)),
        AgentPluginRow::unconfigured(
            Capability,
            hooks_plugin(inputs.hooks, inputs.working_directory),
        ),
        AgentPluginRow::unconfigured(Capability, packages_plugin(inputs.packages)),
    ];
    let (host_rows, host_descriptors) = host.into_iter().unzip::<_, _, Vec<_>, Vec<_>>();
    rows.extend(host_rows);
    rows.extend([
        AgentPluginRow::unconfigured(
            Context,
            workspace_context_plugin(
                inputs.user_instructions_dir,
                inputs.application,
                inputs.workspace,
            ),
        ),
        AgentPluginRow::unconfigured(Context, permission_mode_plugin()),
    ]);
    AgentPluginManifest {
        rows,
        host_descriptors,
    }
}

fn web_config(settings: &AgentPluginSettings) -> Result<WebPluginConfig, String> {
    let agent = &settings.agent;
    Ok(WebPluginConfig {
        provider: match agent.web_search_provider {
            AgentWebSearchProvider::Auto => WebProviderSetting::Auto,
            AgentWebSearchProvider::Duckduckgo => WebProviderSetting::Duckduckgo,
            AgentWebSearchProvider::Searxng => WebProviderSetting::Searxng,
        },
        searxng_base_url: agent.searxng_base_url.clone(),
        proxy: match settings.proxy.clone()? {
            AgentProxy::System => WebProxySetting::System,
            AgentProxy::Disabled => WebProxySetting::Disabled,
            AgentProxy::Explicit(address) => WebProxySetting::Explicit(address),
        },
        jina_reader_enabled: agent.web_fetch_use_jina_reader,
    })
}
