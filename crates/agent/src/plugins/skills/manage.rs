//! `enable_skill` and `disable_skill`: turn one Skill on or off for future
//! sessions through the catalog authority.

use hanafubuki_agent_contracts::ToolResult;
use hanafubuki_agent_skills::SkillId;

use crate::skills::AgentSkillApplication;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkillChange {
    Enable,
    Disable,
}

/// Applies `change`. A refused change is a model-visible result with a typed
/// status, not a failed call.
pub(super) fn change_skill(
    skills: &AgentSkillApplication,
    change: SkillChange,
    skill: &SkillId,
) -> ToolResult {
    let changed = match change {
        SkillChange::Enable => skills.enable(skill),
        SkillChange::Disable => skills.disable(skill),
    };
    match changed {
        Ok(snapshot) => ToolResult::success(serde_json::json!({
            "status": "completed",
            "catalog": snapshot,
            "effectiveFrom": "new_session",
        })),
        Err(reason) => capability_management_error(reason),
    }
}

pub(super) fn capability_management_error(reason: String) -> ToolResult {
    let lower = reason.to_ascii_lowercase();
    let status = if lower.contains("stale") || lower.contains("generation") {
        "stale"
    } else if lower.contains("cleanup")
        || lower.contains("unconfirmed")
        || lower.contains("unknown")
    {
        "unknown_outcome"
    } else if lower.contains("blocked")
        || lower.contains("enabled")
        || lower.contains("referenced")
        || lower.contains("does not exist")
    {
        "blocked"
    } else {
        "failed"
    };
    ToolResult::model_visible_error(serde_json::json!({
        "status": status,
        "error": reason,
    }))
}
