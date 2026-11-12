//! The `skills` plugin: the turn's Skill catalog and the Skill tools over
//! it. When a turn is admitted the plugin freezes the catalog (read from
//! the workspace's repository roots as they are now, narrowed to the Skills
//! whose MCP dependencies are reachable, the others listed as unavailable)
//! on the turn, with the Skills the kernel may select by reference or
//! `$mention` and the tool and visible MCP names a `$mention` names instead;
//! every model request of the turn lists the catalog. A Skill's `SKILL.md`
//! reaches the model once, as the context fragment a `$mention` or a read
//! adopts it with; turning a Skill on or off needs a human's approval, which
//! the plugin's approval rule demands whatever the permission mode.

mod admission;
mod arguments;
mod manage;
mod permission;
mod prompt;
mod read;
mod specs;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentSessionMode, AgentToolError,
    AgentToolHandler, ApprovalRuleRegistry, ArgumentSubjects, BlockingService,
    ContextRendererRegistry, PromptContributorRegistry, Register, ToolDescriptor, ToolEffect,
    ToolFuture, ToolInvocation, ToolPresenterRegistry, ToolRegistry, ToolResult,
    TurnAdmissionRegistry,
};
use hanafubuki_agent_skills::SkillTurnCatalog;
use hanafubuki_agent_tool_contracts::{
    ToolContractError, ToolExecutionClass, ToolExposure, ToolName,
};
use kardia_core::{Context, Inject, PluginDef, Service};

use self::admission::SkillTurnAdmission;
use self::arguments::{parse, SkillCall};
use self::manage::{capability_management_error, change_skill, SkillChange};
use self::permission::{skill_approval_action, SkillManagementRequiresUser};
use self::prompt::{SkillCatalogPrompt, SKILL_CATALOG_RENDERER};
use self::read::{list_skills, read_skill};
use self::specs::spec;
use crate::service::AgentMcpApplication;
use crate::skills::AgentSkillApplication;
use hanafubuki_agent_contracts::verify_tool_approval;

/// Registers the turn admission contributor that freezes the Skill catalog
/// from `skills` (narrowed by `mcp`) with the names a `$mention` names
/// instead of a Skill (the registered tools, visible MCP names), the catalog
/// listing, the four Skill tools and the approval rule of Skill management.
pub(crate) fn skills_plugin(skills: AgentSkillApplication, mcp: AgentMcpApplication) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let skills = skills.clone();
        let mcp = mcp.clone();
        async move {
            // A catalog that did not open is this plugin's failure: the
            // Agent runs without Skills, and the plugin status says why.
            skills.available().map_err(kardia_core::Error::msg)?;
            let admission = ctx.get::<TurnAdmissionRegistry>()?;
            let prompt = ctx.get::<PromptContributorRegistry>()?;
            let tools = ctx.get::<ToolRegistry>()?;
            let rules = ctx.get::<ApprovalRuleRegistry>()?;
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            admission.register(Arc::new(SkillTurnAdmission {
                skills: skills.clone(),
                mcp,
                tools: tools.arc().clone(),
                blocking: blocking.clone(),
            }))?;
            prompt.register(Arc::new(SkillCatalogPrompt))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(SKILL_CATALOG_RENDERER)?;
            for kind in SkillToolKind::ALL {
                let tool = SkillTool::new(kind, skills.clone(), blocking.clone())
                    .map_err(kardia_core::Error::new)?;
                tools.register(Arc::new(tool))?;
            }
            ctx.get::<ToolPresenterRegistry>()?
                .register(Arc::new(ArgumentSubjects::new(&[
                    ("skills_read", "skill"),
                    ("enable_skill", "skill"),
                    ("disable_skill", "skill"),
                ])))?;
            rules.register(Arc::new(SkillManagementRequiresUser))?;
            Ok(())
        }
    })
    .name("skills")
    .inject(
        Inject::new()
            .with(TurnAdmissionRegistry::NAME)
            .with(PromptContributorRegistry::NAME)
            .with(ContextRendererRegistry::NAME)
            .with(ToolRegistry::NAME)
            .with(ToolPresenterRegistry::NAME)
            .with(ApprovalRuleRegistry::NAME)
            .with(BlockingService::NAME),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkillToolKind {
    SkillsList,
    SkillsRead,
    EnableSkill,
    DisableSkill,
}

impl SkillToolKind {
    const ALL: [Self; 4] = [
        Self::SkillsList,
        Self::SkillsRead,
        Self::EnableSkill,
        Self::DisableSkill,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::SkillsList => "skills_list",
            Self::SkillsRead => "skills_read",
            Self::EnableSkill => "enable_skill",
            Self::DisableSkill => "disable_skill",
        }
    }

    const fn action(self) -> &'static str {
        match self {
            Self::SkillsList => "list_skills",
            Self::SkillsRead => "read_skill_resource",
            Self::EnableSkill => "enable_skill",
            Self::DisableSkill => "disable_skill",
        }
    }

    /// Changing which Skills are on is found through `tool_search` and is
    /// unavailable in Plan mode; reading the turn's catalog is neither.
    const fn manages_skills(self) -> bool {
        matches!(self, Self::EnableSkill | Self::DisableSkill)
    }
}

struct SkillTool {
    kind: SkillToolKind,
    skills: AgentSkillApplication,
    blocking: Arc<BlockingService>,
    descriptor: ToolDescriptor,
}

impl SkillTool {
    fn new(
        kind: SkillToolKind,
        skills: AgentSkillApplication,
        blocking: Arc<BlockingService>,
    ) -> Result<Self, ToolContractError> {
        let (description, input_schema) = spec(kind);
        let manages = kind.manages_skills();
        Ok(Self {
            kind,
            skills,
            blocking,
            descriptor: ToolDescriptor {
                name: ToolName::parse(kind.name())?,
                description,
                input_schema,
                exposure: if manages {
                    ToolExposure::Deferred
                } else {
                    ToolExposure::Direct
                },
                execution_class: if manages {
                    ToolExecutionClass::Exclusive
                } else {
                    ToolExecutionClass::ConcurrentRead
                },
                effect: if manages {
                    ToolEffect::Mutating
                } else {
                    ToolEffect::ReadOnly
                },
                action: AgentActivityAction::fixed(kind.action()),
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
        })
    }

    /// The validated call and the approval it needs, recomputed for every
    /// use so `invoke` checks the approval against the call it runs.
    fn prepare(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<(SkillCall, Option<ApprovalAction>), String> {
        let call = parse(self.kind, &invocation.arguments).map_err(|error| error.to_string())?;
        if self.kind.manages_skills() && invocation.context.mode == AgentSessionMode::Plan {
            return Err("capability management mutation is unavailable in Plan mode".to_string());
        }
        let action = skill_approval_action(&invocation.context, &call)?;
        Ok((call, action))
    }

    async fn run(
        &self,
        invocation: &ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> Result<Result<ToolResult, String>, AgentToolError> {
        let (call, required) = match self.prepare(invocation) {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(Err(reason)),
        };
        if let Err(reason) = verify_tool_approval(required.as_ref(), approval.as_ref()) {
            return Ok(Err(reason));
        }
        let skills = self.skills.clone();
        match call {
            SkillCall::List(call) => {
                let catalog = turn_catalog(invocation)?;
                Ok(list_skills(&catalog, &call))
            }
            SkillCall::Read(call) => {
                let catalog = turn_catalog(invocation)?;
                self.blocking
                    .run("agent Skill read", move || {
                        read_skill(&skills, &catalog, &call)
                    })
                    .await
            }
            SkillCall::Enable(skill) => {
                self.blocking
                    .run("agent Skill change", move || {
                        Ok(change_skill(&skills, SkillChange::Enable, &skill))
                    })
                    .await
            }
            SkillCall::Disable(skill) => {
                self.blocking
                    .run("agent Skill change", move || {
                        Ok(change_skill(&skills, SkillChange::Disable, &skill))
                    })
                    .await
            }
        }
    }
}

/// The turn's frozen catalog, which the plugin's admission contributor put
/// in the turn's extensions.
fn turn_catalog(invocation: &ToolInvocation) -> Result<Arc<SkillTurnCatalog>, AgentToolError> {
    invocation
        .turn
        .get_arc::<SkillTurnCatalog>()
        .ok_or_else(|| AgentToolError::Failed("the turn has no Skill catalog".to_string()))
}

impl AgentToolHandler for SkillTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            match self.prepare(invocation).map_err(AgentToolError::Failed)? {
                (_, None) => Ok(PermissionRequirement::NotRequired),
                (_, Some(action)) => Ok(PermissionRequirement::ReviewRequired(action)),
            }
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            let result = self.run(&invocation, approval).await?;
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            match result {
                Ok(result) => Ok(result),
                // A refused Skill change keeps its typed status for the model.
                Err(reason) if self.kind.manages_skills() => {
                    Ok(capability_management_error(reason))
                }
                Err(reason) => Err(AgentToolError::Failed(reason)),
            }
        })
    }
}
