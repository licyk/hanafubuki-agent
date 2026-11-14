//! The `subagents` plugin: Codex's MultiAgentV2 tools (`spawn_agent`,
//! `send_message`, `followup_task`, `wait_agent`, `list_agents`,
//! `interrupt_agent`) over the session driver, and the orchestrator
//! guidance for a conversation that may spawn. An agent is a durable child
//! session of origin `Subagent`, named by its task name, running its turns
//! in the background on the Agent runtime with its parent's permission
//! mode, as the role `spawn_agent`'s `agent_type` names: its parent's scope
//! narrowed by the role's, and the role's instructions. Its Provider, model
//! and reasoning effort are those `spawn_agent` passes, else the role's,
//! else its parent's; what is passed is checked before the agent exists
//! (`models.rs` words the refusals). The plugin provides [`SubagentService`] under its name,
//! which the facade's `subagents` and `interrupt_subagent` resolve per call;
//! with the plugin off, the tools are gone and those calls are unavailable.
//! Its options ([`SubagentsOptions`]) set how many agents run at once and
//! how long a wait lasts.

mod models;
mod options;
mod prompt;
mod service;
mod tools;
mod tree;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ArgumentSubjects, BlockingService, ContextRendererRegistry, PromptContributorRegistry,
    Register, ToolPresenterRegistry, ToolRegistry,
};
use hanafubuki_agent_kernel::{AgentSessionId, ModelReasoningEffort};
use kardia_core::{Context, Inject, PluginDef, Service};
use serde::Serialize;

pub(crate) use self::options::subagents_option_schema;
use self::options::{SubagentLimits, SubagentsOptions};
use self::prompt::{SubagentGuidancePrompt, SUBAGENT_GUIDANCE_RENDERER};
pub(crate) use self::service::SubagentRuns;
pub(crate) use self::service::SubagentService;
use self::tools::{SubagentTool, SubagentToolKind};
use crate::roles::AgentRoleCatalogReader;
use crate::session_driver::AgentSessionDriver;
use crate::wake::Wakes;

/// The plugin's name, under which it provides its [`SubagentService`].
pub(crate) const SUBAGENTS_PLUGIN: &str = "subagents";

/// A subagent of a session, as an interface lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSubagentStatus {
    pub session_id: AgentSessionId,
    pub parent_session_id: AgentSessionId,
    /// Levels below the main conversation (one or two).
    pub depth: usize,
    /// Its task name, the title of its session.
    pub title: String,
    pub status: AgentSubagentState,
    /// The Provider (its id), model and reasoning effort its session runs
    /// on now; no effort is the Provider's default.
    pub provider_id: String,
    pub model: String,
    pub reasoning_effort: Option<ModelReasoningEffort>,
    /// The start of its latest answer, if it has answered.
    pub last_answer: Option<String>,
    /// When its current state began (its turn started, or ended), in Unix
    /// milliseconds; `None` for an agent this Agent instance has not run.
    /// An interface shows how long it has been running from it.
    pub state_since: Option<u64>,
}

/// Whether a subagent runs a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentSubagentState {
    Running,
    /// Its turn ended while its background work or updates remain.
    Waiting,
    /// The person closed the agent.
    Closed,
    /// Its latest turn ended (or was interrupted); it takes follow-up tasks.
    Idle,
    /// Its latest turn failed.
    Failed,
}

/// Registers the subagent tools and the orchestrator guidance over
/// `driver`, with the roles `roles` reads, and provides the
/// [`SubagentService`]. The runs it tracks are `wakes`', which outlive a
/// restart of the plugin, and its agents' turns tell `wakes`' host what
/// they ask of a person.
/// A change of the running-agent limit is taken in place; one of the wait
/// bounds, which `wait_agent`'s spec states, restarts the plugin.
/// The guidance and a spawn read the spawnable roles at each step and call;
/// `spawn_agent`'s spec names none, so it never goes stale.
pub(crate) fn subagents_plugin(
    driver: AgentSessionDriver,
    roles: AgentRoleCatalogReader,
    wakes: Wakes,
) -> PluginDef {
    PluginDef::typed(move |ctx: Context, options: Arc<SubagentsOptions>| {
        let driver = driver.clone();
        let roles = roles.clone();
        let wakes = wakes.clone();
        async move {
            let limits = SubagentLimits::of(&options);
            let service = Arc::new(SubagentService {
                driver,
                runs: wakes.runs.clone(),
                wakes,
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
            ctx.get::<ToolPresenterRegistry>()?
                .register(Arc::new(ArgumentSubjects::new(&[
                    ("spawn_agent", "task_name"),
                    ("send_message", "target"),
                    ("followup_task", "target"),
                    ("interrupt_agent", "target"),
                ])))?;
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
            .with(ToolPresenterRegistry::NAME)
            .with(PromptContributorRegistry::NAME)
            .with(ContextRendererRegistry::NAME)
            .with(BlockingService::NAME),
    )
}

#[cfg(test)]
mod tests;
