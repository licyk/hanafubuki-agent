//! The plugins the Agent runs, each with its configuration derived from the
//! settings, and the host's rows among them. Adding a capability plugin adds
//! one row here; a row derived from the settings follows them while the
//! Agent runs, and any row can be turned off through `disabledPlugins`.

use hanafubuki_agent_config::AgentWebSearchProvider;
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_mcp::McpManager;
use hanafubuki_agent_plugin_execution::execution_plugin;
use hanafubuki_agent_plugin_mcp::mcp_plugin;
use hanafubuki_agent_plugin_web::{
    web_plugin, WebPluginConfig, WebProviderSetting, WebProxySetting,
};
use hanafubuki_agent_plugin_workspace_fs::workspace_fs_plugin;
use hanafubuki_agent_workspace_fs::WorkspaceFsService;
use std::path::PathBuf;
use std::sync::Arc;

use super::{AgentPluginGroup, AgentPluginRow, AgentPluginSettings};
use crate::plugins::{
    memory_plugin, permission_mode_plugin, skills_plugin, titles_plugin, view_image_plugin,
    workspace_context_plugin,
};
use crate::service::AgentMcpApplication;
use crate::skills::AgentSkillApplication;
use crate::titles::AgentTitleApplication;
use crate::{AgentMemoryApplication, AgentProxy, WorkspaceRootsPort};

/// Services the application constructs once and hands to the plugins that
/// adapt them; they are not serializable configuration.
pub(crate) struct AgentPluginInputs {
    /// Where the user's own `AGENTS.md` is read from.
    pub(crate) user_instructions_dir: PathBuf,
    pub(crate) workspace: Arc<dyn WorkspaceRootsPort>,
    pub(crate) files: WorkspaceFsService,
    pub(crate) executions: AgentExecutionService,
    pub(crate) mcp: Arc<McpManager>,
    pub(crate) memory: AgentMemoryApplication,
    pub(crate) skills: AgentSkillApplication,
    /// Narrows the turn's Skill catalog to reachable MCP dependencies.
    pub(crate) mcp_application: AgentMcpApplication,
    pub(crate) titles: AgentTitleApplication,
}

/// The manifest rows in mount order. Each declares its group, which the
/// plugin manager groups them by.
///
/// The host's rows (`host`) mount after the Agent's capability plugins and
/// before its context plugins. A renderer's registration orders its section
/// within its slot, so a host's world-state sections (its UI context) come
/// before the workspace roots, environment and permission mode; only the
/// kernel's own tool catalog omissions, registered with the registries, come
/// first.
pub(crate) fn agent_plugin_manifest(
    inputs: AgentPluginInputs,
    host: Vec<AgentPluginRow>,
) -> Vec<AgentPluginRow> {
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
    ];
    rows.extend(host);
    rows.extend([
        AgentPluginRow::unconfigured(
            Context,
            workspace_context_plugin(inputs.user_instructions_dir, inputs.workspace),
        ),
        AgentPluginRow::unconfigured(Context, permission_mode_plugin()),
    ]);
    rows
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
