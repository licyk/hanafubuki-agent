//! The ceiling on the counters a session persists.

use super::*;

/// The largest value a persisted counter may hold. The kernel seeds these
/// counters from disk and later increments them with `checked_add(1)`; a
/// session past the ceiling is rejected when it is loaded, so no reachable
/// increment can overflow and those expectations are invariants.
pub(in crate::service) const PERSISTED_COUNTER_CEILING: u64 = u64::MAX / 2;

/// Rejects a session that persisted a counter above
/// [`PERSISTED_COUNTER_CEILING`]: its revision and generation, its message
/// and history item ids, its memory contribution revision, its turn's id,
/// generation, steering sequence and model session generation, its
/// compaction's id and generation, its activities' ids, and its history
/// items' ids, revisions and turn ids.
pub(in crate::service) fn validate_persisted_counters(
    runtime: &PersistedAgentRuntimeState,
    model_context: &[AgentHistoryItem],
) -> Result<(), AgentError> {
    let check = |counter: &str, value: u64| {
        if value > PERSISTED_COUNTER_CEILING {
            return Err(AgentError::Persistence(format!(
                "Agent session {} persisted its {counter} as {value}, past the counter ceiling",
                runtime.id
            )));
        }
        Ok(())
    };
    check("revision", runtime.revision.value())?;
    check("generation", runtime.generation.value())?;
    check("next message id", runtime.next_message_id)?;
    check("next history item id", runtime.next_history_item_id)?;
    check(
        "memory contribution revision",
        runtime.memory_contribution.revision,
    )?;
    if let Some(turn) = &runtime.turn {
        check("turn id", turn.id.value())?;
        check("turn generation", turn.generation.value())?;
        check("next steering sequence", turn.next_steering_sequence)?;
        check(
            "model session generation",
            turn.model_session_generation.value(),
        )?;
    }
    if let Some(compaction) = &runtime.compaction {
        check("compaction id", compaction.id.value())?;
        check("compaction generation", compaction.generation.value())?;
    }
    for activity in &runtime.activities {
        check("activity id", activity.id.value())?;
    }
    for item in model_context {
        check("history item id", item.item_id.value())?;
        check("history item revision", item.item_revision)?;
        if let Some(turn_id) = item.turn_id {
            check("history turn id", turn_id.value())?;
        }
        if let ConversationItem::TurnStart { turn_id } = &item.content {
            check("history turn id", turn_id.value())?;
        }
    }
    Ok(())
}
