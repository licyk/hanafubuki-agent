use super::*;

mod activation;
mod helpers;
mod repair;
mod restore;

use repair::repair_after_restart;

fn recorded_turn_ids(record: &SessionRecord) -> impl Iterator<Item = AgentTurnId> + '_ {
    record.runtime.turn.iter().map(|turn| turn.id).chain(
        record
            .conversation
            .history
            .iter()
            .filter_map(|item| match &item.content {
                ConversationItem::TurnStart { turn_id } => Some(*turn_id),
                _ => None,
            }),
    )
}
