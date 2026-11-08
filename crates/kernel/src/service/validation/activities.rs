use crate::budget::AgentBudgetError;
use crate::loop_control::AgentNoProgressError;
use crate::model::AgentError;
use std::collections::HashSet;

pub(in crate::service) fn enforce_size(
    resource: &'static str,
    actual: usize,
    limit: usize,
) -> Result<(), AgentError> {
    if actual > limit {
        Err(AgentError::LimitExceeded { resource, limit })
    } else {
        Ok(())
    }
}

pub(in crate::service) fn validate_bounded_identifier(value: &str) -> Result<(), AgentError> {
    if value.is_empty() || value.len() > 256 {
        Err(AgentError::Protocol(
            "model identifier must contain 1 to 256 bytes".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub(in crate::service) fn bounded_diagnostic(mut message: String) -> String {
    const MAXIMUM_BYTES: usize = 4 * 1024;
    if message.len() <= MAXIMUM_BYTES {
        return message;
    }
    let mut end = MAXIMUM_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message
}

pub(in crate::service) fn validate_persisted_activities(
    activities: &[crate::AgentActivitySnapshot],
) -> Result<(), AgentError> {
    if activities.len() > 64 {
        return Err(AgentError::Persistence(
            "Agent activity snapshot exceeds the retained activity limit".to_string(),
        ));
    }
    let mut ids = HashSet::new();
    for activity in activities {
        if !ids.insert(activity.id) {
            return Err(AgentError::Persistence(
                "Agent activity snapshot contains duplicate ids".to_string(),
            ));
        }
        if activity.finished_at.is_some() != activity.phase.is_terminal() {
            return Err(AgentError::Persistence(
                "Agent activity terminal timestamps are inconsistent".to_string(),
            ));
        }
        if activity.kind == crate::AgentActivityKind::Execution && activity.execution.is_none() {
            return Err(AgentError::Persistence(
                "Agent execution activity is missing its safe execution reference".to_string(),
            ));
        }
        if let Some(output) = &activity.output {
            if output.head.len() > 4
                || output.tail.len() > 8
                || output
                    .head
                    .iter()
                    .chain(output.tail.iter())
                    .any(|line| line.len() > 512)
            {
                return Err(AgentError::Persistence(
                    "Agent execution activity output exceeds its projection bounds".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::service) fn map_budget_error(error: AgentBudgetError) -> AgentError {
    match error {
        AgentBudgetError::Exhausted { resource, limit }
        | AgentBudgetError::InvalidPolicy {
            resource,
            ceiling: limit,
            ..
        } => AgentError::LimitExceeded {
            resource,
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
        },
        AgentBudgetError::Overflow { resource } | AgentBudgetError::Underflow { resource } => {
            AgentError::UsageOverflow { field: resource }
        }
    }
}

pub(in crate::service) fn map_no_progress_error(error: AgentNoProgressError) -> AgentError {
    match error {
        AgentNoProgressError::CounterOverflow => AgentError::UsageOverflow {
            field: "consecutive stalled sampling steps",
        },
        AgentNoProgressError::Exhausted { limit, .. } => AgentError::LimitExceeded {
            resource: "consecutive stalled sampling steps",
            limit: limit as usize,
        },
    }
}
