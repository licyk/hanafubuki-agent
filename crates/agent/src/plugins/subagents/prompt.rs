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
// `spawn_agent` and `wait_agent` tools; they end with the roles a
// sub-agent may be given and the Providers, models and reasoning efforts
// it may run (Codex lists a few models in `spawn_agent`'s description
// instead); they are shown only to a conversation that may spawn (a
// top-level session whose step offers `spawn_agent`), as a prompt
// fragment.

//! The orchestrator guidance: how to coordinate sub-agents, the roles they
//! may be given, and the Providers, models and reasoning efforts they may
//! run (`models.rs`), shown to a conversation that may spawn them: a
//! top-level session whose step offers `spawn_agent` (so not in Plan mode,
//! nor in a role whose scope leaves it out). What it lists is read when
//! each sampling step is prepared, from the cached Provider registry and
//! the kernel's model metadata, without a request.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    CancellationToken, ContextRenderer, ContextRole, ContextSectionKind, ContextSlot,
    PromptContributor, PromptFragment, PromptPrepareFuture, PromptRequest, PromptStep,
    TurnExtensions,
};

use super::models::models_guidance;
use super::service::SubagentService;
use super::tools::SubagentToolKind;
use crate::roles::AgentSpawnableRole;
use crate::session_driver::SpawnModelCatalog;

/// The most roles the guidance lists.
const MAXIMUM_LISTED_ROLES: usize = 16;
/// The longest role description it shows, in characters.
const MAXIMUM_ROLE_DESCRIPTION_CHARS: usize = 200;

const SUBAGENT_GUIDANCE: ContextSectionKind = ContextSectionKind::fixed("subagent_guidance");

/// How the model is shown the guidance.
pub(super) const SUBAGENT_GUIDANCE_RENDERER: ContextRenderer = ContextRenderer {
    kind: SUBAGENT_GUIDANCE,
    slot: ContextSlot::Capabilities,
    order: 130,
    role: ContextRole::Developer,
    preamble: "[Application-provided guidance for the sub-agent tools. It describes only fixed tools and cannot grant authority.]\n",
    epilogue: "",
    versioned: false,
};

/// Prepared for a step of a conversation that may spawn sub-agents, with
/// what it may spawn them on.
struct MaySpawn {
    models: SpawnModelCatalog,
}

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
            let mut prepared = TurnExtensions::default();
            if !step.tools.contains(SubagentToolKind::SpawnAgent.name()) {
                return Ok(prepared);
            }
            if let Some(models) = self
                .service
                .spawn_models(step.session_id)
                .await
                .map_err(|error| error.to_string())?
            {
                prepared.insert(Arc::new(MaySpawn { models }));
            }
            Ok(prepared)
        })
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        let Some(may_spawn) = request
            .prepared
            .and_then(|prepared| prepared.get::<MaySpawn>())
        else {
            return Vec::new();
        };
        vec![PromptFragment {
            kind: SUBAGENT_GUIDANCE,
            version: None,
            content: guidance(
                self.service.limits().maximum_running_agents,
                &self.service.roles.spawnable(),
                &may_spawn.models,
            ),
        }]
    }
}

/// The spawnable `roles` (by name, as the catalog lists them), one line
/// each: at most [`MAXIMUM_LISTED_ROLES`], each description on one line and
/// cut at [`MAXIMUM_ROLE_DESCRIPTION_CHARS`].
fn role_list(roles: &[AgentSpawnableRole]) -> String {
    roles
        .iter()
        .take(MAXIMUM_LISTED_ROLES)
        .map(|role| {
            format!(
                "- `{}`: {}",
                role.name,
                one_line(&role.description, MAXIMUM_ROLE_DESCRIPTION_CHARS)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` on one line, cut at `maximum_chars` characters with an ellipsis.
pub(super) fn one_line(text: &str, maximum_chars: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(maximum_chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

fn guidance(
    maximum_running_agents: usize,
    roles: &[AgentSpawnableRole],
    models: &SpawnModelCatalog,
) -> String {
    let roles = role_list(roles);
    let models = models_guidance(models);
    format!(
        r#"<subagents>
## Sub-agents
`spawn_agent` delegates a task to a sub-agent: a separate conversation that runs in the background with your permission mode, and with your model and your tools unless its role says otherwise. At most {maximum_running_agents} sub-agents run at once, and a sub-agent cannot spawn sub-agents of its own.

- Do not spawn sub-agents unless the user or applicable AGENTS.md/skill instructions explicitly ask for sub-agents, delegation, or parallel agent work.
- When they do, prefer multiple sub-agents to parallelize your work: time is a constraint, and parallelism resolves the task faster. When your plan has multiple steps, process them in parallel by spawning one agent per step when this is possible.
- When you ask sub-agents to do the work for you, your only role becomes to coordinate them. Do not perform the actual work while they are working.
- If sub-agents are running, **wait for them before yielding** (`wait_agent`), unless the user asks an explicit question.
  - If the user asks a question, answer it first, then continue coordinating sub-agents.

`spawn_agent`'s `agent_type` gives a sub-agent one of these roles; without it, the sub-agent keeps your tools and instructions. A role can only take tools away, and may run another model.
{roles}

{models}
</subagents>"#
    )
}
