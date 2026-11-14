//! A session's wake queue: what happened to its background work, kept
//! durably with its runtime until a turn delivers it as one
//! `ConversationItem::WakeEvents`: at a turn's admission (after
//! `TurnStart`), or at a step of the running turn (after its steering).
//! Queuing and taking commit a checkpoint without a revision, as a setting
//! does; delivering appends to history. A queued event is notified
//! (`WakeEventQueued`), which is what wakes an idle session, and so are
//! events that leave the queue (`BackgroundWorkChanged`). What the
//! session's background work holds open is `work.rs`'s.

use super::*;
use crate::{AgentWakeFilter, AgentWakePolicy, AgentWakeQueued, AgentWakeSource};

mod admission;
mod terminal;
mod work;

pub(in crate::service) use terminal::{track_background_terminal, BackgroundTerminals};

impl AgentService {
    /// Queues `source` on `session_id`, durably. A sub-agent's or a
    /// terminal's final event replaces the one already queued for it; a
    /// message past [`crate::MAXIMUM_QUEUED_WAKE_MESSAGES`] unread ones is
    /// refused. With the policy off, only a sub-agent's final status is
    /// queued (`wait_agent` reads it); anything else is dropped. A session
    /// that is archived or gone refuses it, and its caller drops the event.
    /// The observers are told of a queued event once it is durable.
    pub async fn enqueue_wake_event(
        &self,
        session_id: AgentSessionId,
        source: AgentWakeSource,
    ) -> Result<AgentWakeQueued, AgentError> {
        let cell = self.session(session_id)?;
        let (staged, queued) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            if record.status.lifecycle == AgentSessionLifecycle::Archived {
                return Err(AgentError::SessionArchived);
            }
            if record.status.disposing {
                return Err(AgentError::SessionNotFound);
            }
            let delivering = record.runtime.wake_policy == AgentWakePolicy::On;
            if !delivering && !source.is_subagent_status() {
                return Ok(AgentWakeQueued::Disabled);
            }
            if source.is_message() && record.runtime.wake_queue.messages_full() {
                return Err(AgentError::LimitExceeded {
                    resource: "unread wake messages",
                    limit: crate::MAXIMUM_QUEUED_WAKE_MESSAGES,
                });
            }
            let previous = record.rollback_state();
            record.runtime.wake_queue.push(source, now_millis_u64());
            let queued = if delivering && runs_sampling_turn(record) {
                AgentWakeQueued::Delivering
            } else {
                AgentWakeQueued::Waiting
            };
            (StagedPersistence::commit(&cell, record, previous), queued)
        };
        self.persist_staged(staged).await?;
        self.inner
            .registries
            .observers
            .notify(&AgentNotification::WakeEventQueued { session_id });
        Ok(queued)
    }

    /// Removes and returns the events `filter` matches, in queue order:
    /// none taken commits nothing.
    pub async fn take_wake_events(
        &self,
        session_id: AgentSessionId,
        filter: AgentWakeFilter,
    ) -> Result<Vec<crate::AgentWakeEvent>, AgentError> {
        let cell = self.session(session_id)?;
        let (staged, taken) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let taken = record.runtime.wake_queue.take(filter);
            if taken.is_empty() {
                return Ok(taken);
            }
            (StagedPersistence::commit(&cell, record, previous), taken)
        };
        self.persist_staged(staged).await?;
        self.notify_wake_events_left(session_id);
        Ok(taken)
    }

    /// Whether the session's wake events are delivered; returns whether it
    /// changed. Kept in memory: the host gives it again at each activation.
    pub async fn set_session_wake_policy(
        &self,
        session_id: AgentSessionId,
        policy: AgentWakePolicy,
    ) -> Result<bool, AgentError> {
        let cell = self.session(session_id)?;
        // Set only once no commit is staged, so no rollback undoes it.
        let mut record = cell.lock_after_persistence().await?;
        let changed = record.runtime.wake_policy != policy;
        record.runtime.wake_policy = policy;
        Ok(changed)
    }

    /// Delivers the queued wake events to the running turn as one item, at
    /// a step boundary; the sequence of the last one it delivered, if any
    /// (the turn then takes another step). Nothing with the policy off.
    /// What the step's calls reported is applied first, so the end of a
    /// terminal a call's result reported is withdrawn, never delivered.
    pub(super) async fn consume_pending_wake_events(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<u64>, AgentError> {
        let cell = self.session(identity.session_id)?;
        self.flush_execution_progress(&cell).await;
        let (staged, history_event, last) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            exact_turn(record, identity.turn_id, identity.generation)?;
            let Some(last) = record
                .runtime
                .wake_queue
                .events
                .last()
                .map(|event| event.sequence)
                .filter(|_| record.runtime.wake_policy == AgentWakePolicy::On)
            else {
                return Ok(None);
            };
            let previous = record.rollback_state();
            let items = [ConversationItem::WakeEvents {
                events: record.runtime.wake_queue.events.clone(),
            }];
            validate_history_append(record, &items, self.inner.limits)?;
            record.runtime.wake_queue.events.clear();
            append_history(record, &items);
            record.status.revision = record
                .status
                .revision
                .checked_add(1)
                .expect("agent snapshot revision exhausted");
            record.status.updated_at = now_millis();
            let history_event = record_event(
                record,
                identity,
                AgentEventKind::HistoryCommitted {
                    start_index: record.conversation.history.len() - items.len(),
                    item_count: items.len(),
                    usage: None,
                },
                self.inner.limits,
            );
            (
                StagedPersistence::commit(&cell, record, previous),
                history_event,
                last,
            )
        };
        self.persist_staged(staged).await?;
        event_sink.emit(history_event);
        self.notify_wake_events_left(identity.session_id);
        Ok(Some(last))
    }

    /// Tells the observers that events left `session_id`'s queue, once
    /// that is durable.
    pub(super) fn notify_wake_events_left(&self, session_id: AgentSessionId) {
        self.inner
            .registries
            .observers
            .notify(&AgentNotification::BackgroundWorkChanged { session_id });
    }
}

/// Whether the session runs a turn that samples the model, whose steps
/// deliver wake events.
fn runs_sampling_turn(record: &SessionRecord) -> bool {
    record
        .runtime
        .turn
        .as_ref()
        .is_some_and(|turn| !turn.phase.is_terminal() && matches!(turn.kind, TurnKind::Regular))
}

/// Whether a turn ending now would leave wake events it should deliver.
pub(super) fn wake_events_pending(record: &SessionRecord) -> bool {
    record.runtime.wake_policy == AgentWakePolicy::On && !record.runtime.wake_queue.is_empty()
}
