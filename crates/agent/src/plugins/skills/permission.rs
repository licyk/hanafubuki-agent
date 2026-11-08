//! Approval of Skill management: turning a Skill on or off changes the
//! model's future instructions, so it needs approval of the exact change,
//! and the plugin's approval rule puts every `SkillManagement` action to a
//! person, whatever the session's permission mode.

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, PermissionActionDetails,
};
use hanafubuki_agent_contracts::{ApprovalRule, ApprovalRuleVerdict, ToolCallContext};
use hanafubuki_agent_skills::SkillId;
use serde::Serialize;

use super::arguments::SkillCall;

/// The call as the approval digest covers it, serialized as the closed tool
/// set recorded it (`{"type":"enable_skill","skill":…}`).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SkillManagementCall<'a> {
    EnableSkill { skill: &'a SkillId },
    DisableSkill { skill: &'a SkillId },
}

/// The approval `call` needs: none for a read, the exact change for Skill
/// management.
pub(super) fn skill_approval_action(
    context: &ToolCallContext,
    call: &SkillCall,
) -> Result<Option<ApprovalAction>, String> {
    let (operation, recorded) = match call {
        SkillCall::List(_) | SkillCall::Read(_) => return Ok(None),
        SkillCall::Enable(skill) => ("enable", SkillManagementCall::EnableSkill { skill }),
        SkillCall::Disable(skill) => ("disable", SkillManagementCall::DisableSkill { skill }),
    };
    let skill = match &recorded {
        SkillManagementCall::EnableSkill { skill }
        | SkillManagementCall::DisableSkill { skill } => skill.to_string(),
    };
    let canonical = serde_json::to_vec(&(
        "agent-skill-management-v4",
        context.session_id,
        context.generation,
        context.call_id.as_str(),
        &recorded,
    ))
    .map_err(|error| error.to_string())?;
    ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::SkillManagement,
        ApprovalRiskClass::Critical,
        ApprovalOrigin::Model,
        format!("{operation} Skill {skill}"),
        &canonical,
    )
    .and_then(|action| {
        action.with_details(PermissionActionDetails::SkillManagement {
            operation: operation.to_string(),
            skill: Some(skill),
            effective_from: "new_session".to_string(),
        })
    })
    .map(Some)
    .map_err(|error| error.to_string())
}

/// A person decides every `SkillManagement` action: neither `FullAccess` nor
/// the automatic reviewer may approve a change to the model's instructions.
pub(super) struct SkillManagementRequiresUser;

impl ApprovalRule for SkillManagementRequiresUser {
    fn id(&self) -> &str {
        "skills.management"
    }

    fn evaluate(&self, action: &ApprovalAction) -> Option<ApprovalRuleVerdict> {
        (action.kind == ApprovalActionKind::SkillManagement)
            .then_some(ApprovalRuleVerdict::RequireUser)
    }
}
