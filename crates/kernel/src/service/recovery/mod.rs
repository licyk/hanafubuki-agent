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

impl AgentService {
    fn advance_recovered_ids(&self, record: &SessionRecord) {
        for turn_id in recorded_turn_ids(record) {
            self.inner.next_turn_id.fetch_max(
                turn_id
                    .value()
                    .checked_add(1)
                    .expect("agent turn id exhausted"),
                Ordering::AcqRel,
            );
        }
        if let Some(compaction) = &record.runtime.compaction {
            self.inner.next_compaction_id.fetch_max(
                compaction
                    .snapshot
                    .id
                    .value()
                    .checked_add(1)
                    .expect("agent compaction id exhausted"),
                Ordering::AcqRel,
            );
        }
    }
}
