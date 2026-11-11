//! The `subagents` plugin: Codex's MultiAgentV2 tools (`spawn_agent`,
//! `send_message`, `followup_task`, `wait_agent`, `list_agents`,
//! `interrupt_agent`) over the session driver, and the orchestrator
//! guidance for a conversation that may spawn. An agent is a durable child
//! session of origin `Subagent`, named by its task name, running its turns
//! in the background on the Agent runtime with its parent's permission
//! mode, as the role `spawn_agent`'s `agent_type` names: its parent's scope
//! narrowed by the role's, and the role's instructions and model (else its
//! parent's). The plugin provides [`SubagentService`] under its name,
//! which the facade's `subagents` and `interrupt_subagent` resolve per call;
//! with the plugin off, the tools are gone and those calls are unavailable.
//! Its options ([`SubagentsOptions`]) set how many agents run at once and
//! how long a wait lasts.

mod options;
mod prompt;
mod service;
mod tools;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    BlockingService, ContextRendererRegistry, PromptContributorRegistry, Register, ToolRegistry,
};
use hanafubuki_agent_kernel::AgentSessionId;
use kardia_core::{Context, Inject, PluginDef, Service};
use serde::Serialize;

pub(crate) use self::options::subagents_option_schema;
use self::options::{SubagentLimits, SubagentsOptions};
use self::prompt::{SubagentGuidancePrompt, SUBAGENT_GUIDANCE_RENDERER};
use self::service::SubagentRuns;
pub(crate) use self::service::SubagentService;
use self::tools::{SubagentTool, SubagentToolKind};
use crate::roles::AgentRoleCatalogReader;
use crate::session_driver::AgentSessionDriver;

/// The plugin's name, under which it provides its [`SubagentService`].
pub(crate) const SUBAGENTS_PLUGIN: &str = "subagents";

/// A subagent of a session, as an interface lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSubagentStatus {
    pub session_id: AgentSessionId,
    /// Its task name, the title of its session.
    pub title: String,
    pub status: AgentSubagentState,
    /// The start of its latest answer, if it has answered.
    pub last_answer: Option<String>,
}

/// Whether a subagent runs a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentSubagentState {
    Running,
    /// Its latest turn ended (or was interrupted); it takes follow-up tasks.
    Idle,
    /// Its latest turn failed.
    Failed,
}

/// Registers the subagent tools and the orchestrator guidance over
/// `driver`, with the roles `roles` reads, and provides the
/// [`SubagentService`]. The runs it tracks outlive a restart of the plugin.
/// A change of the running-agent limit is taken in place; one of the wait
/// bounds, which `wait_agent`'s spec states, restarts the plugin.
/// The guidance and a spawn read the spawnable roles at each step and call;
/// `spawn_agent`'s spec names none, so it never goes stale.
pub(crate) fn subagents_plugin(
    driver: AgentSessionDriver,
    roles: AgentRoleCatalogReader,
) -> PluginDef {
    let runs = SubagentRuns::default();
    PluginDef::typed(move |ctx: Context, options: Arc<SubagentsOptions>| {
        let driver = driver.clone();
        let roles = roles.clone();
        let runs = runs.clone();
        async move {
            let limits = SubagentLimits::of(&options);
            let service = Arc::new(SubagentService {
                driver,
                runs,
                blocking: ctx.get::<BlockingService>()?.arc().clone(),
                roles,
                limits: parking_lot::Mutex::new(limits),
            });
            let updated = service.clone();
            ctx.on_update(move |options: &SubagentsOptions| {
                let limits = SubagentLimits::of(options);
                let mut current = updated.limits.lock();
                if limits.waits != current.waits {
                    return false;
                }
                *current = limits;
                true
            })?;
            let tools = ctx.get::<ToolRegistry>()?;
            for kind in SubagentToolKind::ALL {
                let tool =
                    SubagentTool::new(kind, service.clone()).map_err(kardia_core::Error::new)?;
                tools.register(Arc::new(tool))?;
            }
            ctx.get::<PromptContributorRegistry>()?
                .register(Arc::new(SubagentGuidancePrompt {
                    service: service.clone(),
                }))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(SUBAGENT_GUIDANCE_RENDERER)?;
            ctx.provide_arc_as(SUBAGENTS_PLUGIN, service)?;
            Ok(())
        }
    })
    .name(SUBAGENTS_PLUGIN)
    .provides([SUBAGENTS_PLUGIN])
    .inject(
        Inject::new()
            .with(ToolRegistry::NAME)
            .with(PromptContributorRegistry::NAME)
            .with(ContextRendererRegistry::NAME)
            .with(BlockingService::NAME),
    )
}

#[cfg(test)]
mod tests;
