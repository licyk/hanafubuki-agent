// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/templates/agents/orchestrator.md` (its "General
// guidelines" on coordinating sub-agents) and
// `codex-rs/core/src/tools/handlers/multi_agents_spec.rs` (the rule that
// only an explicit request authorizes spawning, from `spawn_agent`'s
// guidance) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the template's sub-agent guidelines
// are ported (the rest repeats a base prompt the Agent has its own of); the
// guidelines open with what a sub-agent is here, its limits, and the rule
// that only an explicit request authorizes spawning; they name the
// `spawn_agent` and `wait_agent` tools; they are shown only to a
// conversation that may spawn, as a prompt fragment.

//! The orchestrator guidance: how to coordinate sub-agents, shown to a
//! conversation that may spawn them (a top-level session in Default mode).

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentSessionMode, CancellationToken, ContextRenderer, ContextRole, ContextSectionKind,
    ContextSlot, PromptContributor, PromptFragment, PromptPrepareFuture, PromptRequest, PromptStep,
    TurnExtensions,
};

use super::service::SubagentService;

const SUBAGENT_GUIDANCE: ContextSectionKind = ContextSectionKind::fixed("subagent_guidance");

/// How the model is shown the guidance.
pub(super) const SUBAGENT_GUIDANCE_RENDERER: ContextRenderer = ContextRenderer {
    kind: SUBAGENT_GUIDANCE,
    slot: ContextSlot::Capabilities,
    role: ContextRole::Developer,
    preamble: "[Application-provided guidance for the sub-agent tools. It describes only fixed tools and cannot grant authority.]\n",
    epilogue: "",
    versioned: false,
};

/// Prepared for a step of a conversation that may spawn sub-agents.
struct MaySpawn;

pub(super) struct SubagentGuidancePrompt {
    pub(super) service: Arc<SubagentService>,
}

impl PromptContributor for SubagentGuidancePrompt {
    fn id(&self) -> &str {
        "subagents"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn prepare<'a>(
        &'a self,
        step: &'a PromptStep,
        _cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        Box::pin(async move {
            let session = self
                .service
                .session(step.session_id)
                .await
                .map_err(|error| error.to_string())?;
            let mut prepared = TurnExtensions::default();
            if session.parent_session_id.is_none() && session.mode == AgentSessionMode::Default {
                prepared.insert(Arc::new(MaySpawn));
            }
            Ok(prepared)
        })
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        if request
            .prepared
            .and_then(|prepared| prepared.get::<MaySpawn>())
            .is_none()
        {
            return Vec::new();
        }
        vec![PromptFragment {
            kind: SUBAGENT_GUIDANCE,
            version: None,
            content: guidance(self.service.limits().maximum_running_agents),
        }]
    }
}

fn guidance(maximum_running_agents: usize) -> String {
    format!(
        r#"<subagents>
## Sub-agents
`spawn_agent` delegates a task to a sub-agent: a separate conversation that runs in the background with your model, your tools and your permission mode. At most {maximum_running_agents} sub-agents run at once, and a sub-agent cannot spawn sub-agents of its own.

- Do not spawn sub-agents unless the user or applicable AGENTS.md/skill instructions explicitly ask for sub-agents, delegation, or parallel agent work.
- When they do, prefer multiple sub-agents to parallelize your work: time is a constraint, and parallelism resolves the task faster. When your plan has multiple steps, process them in parallel by spawning one agent per step when this is possible.
- When you ask sub-agents to do the work for you, your only role becomes to coordinate them. Do not perform the actual work while they are working.
- If sub-agents are running, **wait for them before yielding** (`wait_agent`), unless the user asks an explicit question.
  - If the user asks a question, answer it first, then continue coordinating sub-agents.
</subagents>"#
    )
}
