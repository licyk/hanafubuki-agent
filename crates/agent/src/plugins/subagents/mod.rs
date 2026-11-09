//! The `subagents` plugin: Codex's MultiAgentV2 tools (`spawn_agent`,
//! `send_message`, `followup_task`, `wait_agent`, `list_agents`,
//! `interrupt_agent`) over the session driver, and the orchestrator
//! guidance for a conversation that may spawn. An agent is a durable child
//! session of origin `Subagent`, named by its task name, running its turns
//! in the background on the Agent runtime with its parent's model and
//! permission mode. The plugin provides [`SubagentService`] under its name,
//! which the facade's `subagents` and `interrupt_subagent` resolve per call;
//! with the plugin off, the tools are gone and those calls are unavailable.

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

use self::prompt::{SubagentGuidancePrompt, SUBAGENT_GUIDANCE_RENDERER};
use self::service::SubagentRuns;
pub(crate) use self::service::SubagentService;
use self::tools::{SubagentTool, SubagentToolKind};
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
/// `driver`, and provides the [`SubagentService`]. The runs it tracks
/// outlive a restart of the plugin.
pub(crate) fn subagents_plugin(driver: AgentSessionDriver) -> PluginDef {
    let runs = SubagentRuns::default();
    PluginDef::new(move |ctx: Context, _| {
        let driver = driver.clone();
        let runs = runs.clone();
        async move {
            let service = Arc::new(SubagentService {
                driver,
                runs,
                blocking: ctx.get::<BlockingService>()?.arc().clone(),
            });
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
