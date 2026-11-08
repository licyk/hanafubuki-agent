use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction};

/// Checks a consumed approval against the action a call requires, recomputed
/// when the call runs: a read takes none, and a mutation only its exact one.
pub fn verify_tool_approval(
    required: Option<&ApprovalAction>,
    approval: Option<&AuthorizedAction>,
) -> Result<(), String> {
    match (required, approval) {
        (None, None) => Ok(()),
        (Some(expected), Some(approved)) if approved.action() == expected => Ok(()),
        (None, Some(_)) => Err("read-only Agent tool received an unrelated approval".to_string()),
        (Some(_), None) => {
            Err("Agent mutation tool requires an exact consumed approval".to_string())
        }
        (Some(_), Some(_)) => {
            Err("Agent approval does not match the requested mutation".to_string())
        }
    }
}
