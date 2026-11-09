//! The state a session starts with when its window comes from elsewhere: a
//! fork before a turn (and so `edit_from_turn`), a child forked before its
//! parent's running turn, a session recovered without a readable state.

use hanafubuki_agent_compression::{adopt_fork_state, rebuild_state, Config, COMPRESS_TOOL_NAME};

use super::*;

/// The state for a window of `history` made from a session whose state was
/// `parent`. The parent's blocks whose messages are all in `history` are
/// adopted and its refs kept (item ids survive a fork, so its message ids
/// do). Without a parent state the blocks are rebuilt by replaying the
/// `compress` calls in `history`, when there are any and the window was
/// never compacted. `None` when there is nothing to keep.
pub(in crate::service) fn inherited_state(
    settings: &SessionRecord,
    parent: Option<&AgentCompressionState>,
    history: &[Arc<AgentHistoryItem>],
) -> Option<Arc<AgentCompressionState>> {
    if parent.is_none() && !replayable(history) {
        return None;
    }
    let inherited = catch_panic(|| {
        let log = MessageLog::new(history);
        if let Some(parent) = parent {
            let present = log
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect();
            return adopt_fork_state(parent, &present);
        }
        let rebuilt = rebuild_state(
            AgentCompressionState::default(),
            &log.messages,
            &replay_config(hierarchical_config(settings)),
            now_millis_u64(),
        );
        log::info!(
            "Agent session compression state rebuilt from the history: {} block(s)",
            rebuilt.blocks_rebuilt
        );
        rebuilt.state
    });
    match inherited {
        Ok(state) => Some(Arc::new(state)),
        Err(message) => {
            log::warn!(
                "Agent session compression state not inherited: {}",
                bounded_diagnostic(message)
            );
            None
        }
    }
}

/// The window holds `compress` calls whose refs a replay numbers as they
/// were. After a compaction the refs had gone on from where the compacted
/// window left them, so a replay over what is left would number them anew
/// and could fold the wrong messages.
fn replayable(history: &[Arc<AgentHistoryItem>]) -> bool {
    let mut calls = false;
    for item in history {
        match &item.content {
            ConversationItem::CompactionSummary { .. }
            | ConversationItem::ProviderCompaction { .. } => return false,
            ConversationItem::ToolCall { name, .. } if name == COMPRESS_TOOL_NAME => calls = true,
            _ => {}
        }
    }
    calls
}

/// The calls replayed were applied once, under limits the replay may not
/// know (the turn's tuning is gone), so only their content is checked again.
fn replay_config(mut config: Config) -> Config {
    config.compress.min_compress_range = 0;
    config.compress.min_summary_length = 0;
    config.compress.max_summary_length = 0;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}
