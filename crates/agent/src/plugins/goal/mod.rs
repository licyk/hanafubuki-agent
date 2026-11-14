//! The `goal` plugin: a session's goal, an objective the session keeps
//! working toward across final answers within its budgets. The model reads
//! and sets it with `get_goal`, `create_goal` and `update_goal`; each turn
//! starts with it in the `goal` world-state section; and a continuation hook
//! keeps a turn going while the goal is active, unless the session is woken
//! for background work still running, whose woken turn goes on with it. It provides [`GoalService`]
//! under its name for the facade's goal calls; with the plugin off or
//! failed, only goals are unavailable (a goal already set stays stored with
//! its session).

mod context;
mod runtime;
mod templates;
mod tools;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentContinuationContext, AgentContinuationHook, AgentContinuationHookDecision,
    AgentContinuationHookError, AgentContinuationHookFailurePolicy, AgentContinuationHookFuture,
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    AgentContinuationHookRegistration, ArgumentSubjects, BlockingService, CancellationToken,
    ContextRegistry, ContextRendererRegistry, ContinuationHookRegistry, Register,
    ToolPresenterRegistry, ToolRegistry,
};
use hanafubuki_agent_kernel::{AgentError, AgentGoal, AgentGoalOrigin, AgentSessionId};
use kardia_core::{Context, Inject, PluginDef, Service};
use sha2::{Digest, Sha256};

pub use runtime::{AgentGoalBudgets, MAXIMUM_GOAL_OBJECTIVE_CHARS};

use crate::goals::AgentGoalApplication;
use context::{goal_context_renderer, GoalContext};
use runtime::GoalRuntime;

/// The plugin's name, under which it provides its [`GoalService`]; also the
/// provider id of its continuation hook.
pub(crate) const GOAL_PLUGIN: &str = "goal";

/// After the user's own hooks (`hooks`, priority 0), so a Stop hook that
/// ends the turn ends it before the goal keeps it going.
const GOAL_HOOK_PRIORITY: i32 = 100;

/// Registers the goal tools, the goal context and the continuation hook
/// over `goals`, and provides the [`GoalService`].
pub(crate) fn goal_plugin(goals: AgentGoalApplication) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let goals = goals.clone();
        async move {
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            let runtime = Arc::new(GoalRuntime::new(goals, blocking));
            let registry = ctx.get::<ToolRegistry>()?;
            for tool in tools::goal_tools(runtime.clone()).map_err(kardia_core::Error::new)? {
                registry.register(Arc::new(tool))?;
            }
            ctx.get::<ToolPresenterRegistry>()?
                .register(Arc::new(ArgumentSubjects::new(&[
                    ("create_goal", "objective"),
                    ("update_goal", "status"),
                ])))?;
            ctx.get::<ContextRegistry>()?
                .register(Arc::new(GoalContext(runtime.clone())))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(goal_context_renderer())?;
            ctx.get::<ContinuationHookRegistry>()?
                .register(AgentContinuationHookRegistration {
                    provider_id: AgentContinuationHookProviderId::parse(GOAL_PLUGIN.to_string())
                        .map_err(kardia_core::Error::msg)?,
                    hook_id: AgentContinuationHookId::parse("continue".to_string())
                        .map_err(kardia_core::Error::msg)?,
                    generation: AgentContinuationHookGeneration(
                        Sha256::digest(b"hanafubuki-agent-plugin-goal/1").into(),
                    ),
                    priority: GOAL_HOOK_PRIORITY,
                    // A goal that cannot be read must not fail the turn.
                    failure_policy: AgentContinuationHookFailurePolicy::Ignore,
                    hook: Arc::new(GoalContinuation(runtime.clone())),
                })?;
            ctx.provide_arc_as(GOAL_PLUGIN, Arc::new(GoalService { runtime }))?;
            Ok(())
        }
    })
    .name(GOAL_PLUGIN)
    .provides([GOAL_PLUGIN])
    .inject(
        Inject::new()
            .with(BlockingService::NAME)
            .with(ToolRegistry::NAME)
            .with(ToolPresenterRegistry::NAME)
            .with(ContextRegistry::NAME)
            .with(ContextRendererRegistry::NAME)
            .with(ContinuationHookRegistry::NAME),
    )
}

/// Keeps a turn going while its session's goal is active; a session whose
/// goal is not active is neither asked nor reported. A turn whose session
/// is woken for background work still running passes: it ends, and the
/// woken turn goes on with the goal.
struct GoalContinuation(Arc<GoalRuntime>);

impl AgentContinuationHook for GoalContinuation {
    fn applies(&self, context: &AgentContinuationContext) -> bool {
        self.0.active(context.session_id)
    }

    fn evaluate<'a>(
        &'a self,
        context: AgentContinuationContext,
        _cancellation: CancellationToken,
    ) -> AgentContinuationHookFuture<'a> {
        Box::pin(async move {
            if !context.background.is_empty() {
                return Ok(AgentContinuationHookDecision::Pass);
            }
            match self.0.continuation(context.session_id).await {
                Ok(Some(fragment)) => Ok(AgentContinuationHookDecision::Continue {
                    fragments: vec![fragment],
                }),
                Ok(None) => Ok(AgentContinuationHookDecision::Pass),
                Err(error) => Err(AgentContinuationHookError::Failed(error.to_string())),
            }
        })
    }
}

/// What the `goal` row provides: the facade's goal calls.
pub(crate) struct GoalService {
    runtime: Arc<GoalRuntime>,
}

impl GoalService {
    pub(crate) async fn goal(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<AgentGoal>, AgentError> {
        self.runtime.goal(session_id).await
    }

    /// A new active goal set by the user, replacing the session's.
    pub(crate) async fn set(
        &self,
        session_id: AgentSessionId,
        objective: String,
        budgets: AgentGoalBudgets,
    ) -> Result<AgentGoal, AgentError> {
        self.runtime.ensure_persistent(session_id).await?;
        self.runtime
            .create(session_id, &objective, budgets, AgentGoalOrigin::User)
            .await
    }

    pub(crate) async fn clear(&self, session_id: AgentSessionId) -> Result<bool, AgentError> {
        self.runtime.clear(session_id).await
    }

    pub(crate) async fn pause(&self, session_id: AgentSessionId) -> Result<AgentGoal, AgentError> {
        self.runtime.pause(session_id).await
    }

    pub(crate) async fn resume(&self, session_id: AgentSessionId) -> Result<AgentGoal, AgentError> {
        self.runtime.ensure_persistent(session_id).await?;
        self.runtime.resume(session_id).await
    }
}

#[cfg(test)]
mod tests;
