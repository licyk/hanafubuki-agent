use super::*;

mod child;
mod configuration;
mod context;
mod create;
mod events;
mod fork;
mod goal;
mod memory;
mod query;
mod title;
mod workspace;

fn ensure_head_fork_idle(record: &SessionRecord) -> Result<(), AgentError> {
    if record
        .runtime
        .turn
        .as_ref()
        .is_some_and(|turn| !turn.phase.is_terminal())
    {
        return Err(AgentError::TurnAlreadyRunning);
    }
    if record
        .runtime
        .compaction
        .as_ref()
        .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
    {
        return Err(AgentError::CompactionAlreadyRunning);
    }
    Ok(())
}

struct ForkMaterial {
    history: Vec<AgentHistoryItem>,
    restored_prompt: String,
    restored_images: Vec<crate::AgentImageAttachmentRef>,
}

struct ForkScan {
    before_turn_id: AgentTurnId,
    checkpoint_key: Option<(AgentSessionId, AgentHistoryItemId)>,
    checkpoint_replacement: Vec<AgentHistoryItem>,
    history: Vec<AgentHistoryItem>,
    history_overflowed: bool,
    maximum_history_items: Option<usize>,
    target_seen: bool,
    restored: Option<(String, Vec<crate::AgentImageAttachmentRef>)>,
}

impl ForkScan {
    fn new(
        source: &SessionRecord,
        before_turn_id: AgentTurnId,
        maximum_history_items: Option<usize>,
    ) -> Self {
        let (checkpoint_key, checkpoint_replacement) =
            match &source.conversation.model_context_boundary {
                AgentModelContextBoundary::Full => (None, Vec::new()),
                AgentModelContextBoundary::Compacted { checkpoint } => (
                    Some((source.session_id, checkpoint.checkpoint_item_id)),
                    checkpoint.replacement_history.clone(),
                ),
            };
        Self {
            before_turn_id,
            checkpoint_key,
            checkpoint_replacement,
            history: Vec::new(),
            history_overflowed: false,
            maximum_history_items,
            target_seen: false,
            restored: None,
        }
    }

    fn extend(&mut self, items: impl IntoIterator<Item = AgentHistoryItem>, transcript: bool) {
        for item in items {
            if self.restored.is_some() {
                break;
            }
            if self.target_seen {
                if let ConversationItem::UserMessage { text, images } = item.content {
                    self.restored = Some((text, images));
                } else if matches!(item.content, ConversationItem::TurnStart { .. }) {
                    break;
                }
                continue;
            }
            if matches!(
                item.content,
                ConversationItem::TurnStart { turn_id } if turn_id == self.before_turn_id
            ) {
                self.target_seen = true;
                continue;
            }
            if self
                .checkpoint_key
                .is_some_and(|key| key == (item.source_session_id, item.item_id))
            {
                self.history = self.checkpoint_replacement.clone();
                self.history_overflowed = self
                    .maximum_history_items
                    .is_some_and(|limit| self.history.len() > limit);
                continue;
            }
            // Transcript summaries mark completed compactions for display. The
            // model replacement is installed only by the checkpoint above.
            if transcript && matches!(item.content, ConversationItem::CompactionSummary { .. }) {
                continue;
            }
            if self
                .maximum_history_items
                .is_none_or(|limit| self.history.len() < limit)
            {
                self.history.push(item);
            } else {
                self.history_overflowed = true;
            }
        }
    }

    fn finish_if_found(&mut self) -> Result<Option<ForkMaterial>, AgentError> {
        let Some((restored_prompt, restored_images)) = self.restored.take() else {
            return Ok(None);
        };
        if let Some(limit) = self
            .maximum_history_items
            .filter(|_| self.history_overflowed)
        {
            return Err(AgentError::LimitExceeded {
                resource: "fork model history items",
                limit,
            });
        }
        Ok(Some(ForkMaterial {
            history: std::mem::take(&mut self.history),
            restored_prompt,
            restored_images,
        }))
    }
}
mod skills;
