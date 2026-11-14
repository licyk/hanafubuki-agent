use super::*;
use crate::AgentHistoryFollowUp;

impl AgentService {
    /// The cell of a live session.
    pub(super) fn session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Arc<SessionCell>, AgentError> {
        self.inner.sessions.get(session_id)
    }

    /// Drain loaded session writers before a raw diagnostic filesystem copy.
    /// Off the async runtime only: it waits for each session's staged commit
    /// and flushes its writer.
    pub fn flush_for_diagnostics(&self) -> Result<(), AgentError> {
        for cell in self.inner.sessions.cells() {
            match cell.lock_after_persistence_blocking() {
                Ok(guard) => drop(guard),
                Err(AgentError::SessionNotFound) => continue,
                Err(error) => return Err(error),
            }
            self.inner
                .history_store
                .flush_session(cell.session_id())
                .map_err(history_store_error)?;
        }
        Ok(())
    }

    /// Commits `cell`'s current state as its first record (`create`) or as a
    /// checkpoint, on the calling thread.
    pub(super) fn persist_current_conversation(
        &self,
        cell: &Arc<SessionCell>,
        create: bool,
    ) -> Result<(), AgentError> {
        let commit = {
            let mut record = cell.lock_after_persistence_blocking()?;
            conversation_commit(cell.session_id(), &mut record)
        };
        let revision = commit.runtime.revision.value();
        let result = if create {
            self.inner.history_store.create_session(commit)
        } else {
            self.inner.history_store.append_commit(commit)
        };
        if result.is_ok() {
            self.hand_off(publish_persisted_revision(cell, revision));
        }
        result.map_err(history_store_error)
    }

    pub(super) fn persist_current_fork(
        &self,
        cell: &Arc<SessionCell>,
        source_session_id: AgentSessionId,
        before_turn_id: AgentTurnId,
    ) -> Result<(), AgentError> {
        let commit = {
            let mut record = cell.lock_after_persistence_blocking()?;
            conversation_commit(cell.session_id(), &mut record)
        };
        let base = self
            .inner
            .history_store
            .create_fork(source_session_id, before_turn_id, commit)
            .map_err(history_store_error)?;
        self.hand_off(publish_fork_base(cell, base)?);
        Ok(())
    }

    pub(super) fn persist_current_head_fork(
        &self,
        cell: &Arc<SessionCell>,
        source_session_id: AgentSessionId,
        source_revision: AgentRevision,
    ) -> Result<(), AgentError> {
        let commit = {
            let mut record = cell.lock_after_persistence_blocking()?;
            conversation_commit(cell.session_id(), &mut record)
        };
        let base = self
            .inner
            .history_store
            .create_fork_at_head(source_session_id, source_revision, commit)
            .map_err(history_store_error)?;
        self.hand_off(publish_fork_base(cell, base)?);
        Ok(())
    }

    /// Queues a deferred commit that a settlement staged off the history
    /// writer; its acknowledgement publishes it.
    pub(super) fn hand_off(&self, follow_up: Option<AgentHistoryFollowUp>) {
        if let Some(AgentHistoryFollowUp { commit, ack }) = follow_up {
            self.inner.history_store.enqueue_commit(commit, ack);
        }
    }

    pub(super) async fn commit_history(
        &self,
        identity: TurnIdentity,
        items: &[ConversationItem],
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.commit_history_with(
            identity,
            |_| {
                (
                    std::borrow::Cow::Borrowed(items),
                    super::compression::CompressionChange::default(),
                )
            },
            event_sink,
        )
        .await
    }

    /// Commits the items `prepare` builds from the record under the commit's
    /// lock, with the compression state it returns, if any, in the same
    /// commit (and the content store in the record, written after it).
    pub(super) async fn commit_history_with<'a>(
        &self,
        identity: TurnIdentity,
        prepare: impl FnOnce(
            &SessionRecord,
        ) -> (
            std::borrow::Cow<'a, [ConversationItem]>,
            super::compression::CompressionChange,
        ),
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let (staged, events) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let (items, compression) = prepare(record);
            let items = &*items;
            let previous = record.rollback_state();
            let (tool_output_bytes, hook_fragment_bytes, hook_fragment_items) = items
                .iter()
                .try_fold((0usize, 0usize, 0usize), |totals, item| match item {
                    ConversationItem::ToolOutput { output, images, .. } => {
                        let output_bytes = serde_json::to_vec(output)
                            .map_err(|error| AgentError::Protocol(error.to_string()))?
                            .len();
                        let image_bytes = images.iter().fold(0usize, |bytes, image| {
                            bytes.saturating_add(
                                usize::try_from(image.byte_length).unwrap_or(usize::MAX),
                            )
                        });
                        Ok::<_, AgentError>((
                            totals
                                .0
                                .saturating_add(output_bytes)
                                .saturating_add(image_bytes),
                            totals.1,
                            totals.2,
                        ))
                    }
                    ConversationItem::ContinuationContext { content, .. } => Ok((
                        totals.0,
                        totals.1.saturating_add(content.len()),
                        totals.2.saturating_add(1),
                    )),
                    _ => Ok(totals),
                })?;
            let policy = record.settings.budget_policy;
            let mut next_ledger = exact_turn_mut(record, identity)?.budget_ledger;
            if tool_output_bytes > 0 {
                next_ledger
                    .record_tool_output_bytes(tool_output_bytes, policy)
                    .map_err(map_budget_error)?;
            }
            if hook_fragment_bytes > 0 {
                next_ledger
                    .record_hook_fragment_bytes(hook_fragment_bytes, policy)
                    .map_err(map_budget_error)?;
            }
            if hook_fragment_items > 0 {
                next_ledger
                    .admit_pending_input_items(hook_fragment_items, policy)
                    .map_err(map_budget_error)?;
            }
            validate_history_append(record, items, self.inner.limits)?;
            exact_turn_mut(record, identity)?.budget_ledger = next_ledger;
            let start_index = record.conversation.history.len();
            append_history(record, items);
            if let Some(state) = compression.state {
                record.runtime.compression = Some(Arc::new(state));
            }
            if let Some(store) = compression.content_store {
                record
                    .runtime
                    .compression_runtime
                    .keep_content_store(Arc::new(store));
            }
            let mut events = vec![record_event(
                record,
                identity,
                AgentEventKind::HistoryCommitted {
                    start_index,
                    item_count: items.len(),
                    usage: None,
                },
                self.inner.limits,
            )];
            if let Some((call_id, fold)) = compression.fold {
                events.push(record_event(
                    record,
                    identity,
                    AgentEventKind::ContextCompressed {
                        call_id,
                        blocks_created: fold.blocks_created,
                        tokens_compressed: fold.tokens_compressed,
                        tier: fold.tier,
                    },
                    self.inner.limits,
                ));
            }
            (StagedPersistence::commit(&cell, record, previous), events)
        };
        self.persist_staged(staged).await?;
        for event in events {
            event_sink.emit(event);
        }
        Ok(())
    }
}

/// Publishes `revision` as durable: the record's staged commit settled.
/// Returns the deferred commit staged in its place, which the caller hands
/// to the history store.
#[must_use]
pub(super) fn publish_persisted_revision(
    cell: &Arc<SessionCell>,
    revision: u64,
) -> Option<AgentHistoryFollowUp> {
    let follow_up = cell.lock().ok().and_then(|mut record| {
        if record.status.persisting_revision == Some(revision) {
            record.status.persisting_revision = None;
        }
        record.stage_deferred(cell)
    });
    cell.notify_persisted();
    follow_up
}

/// Rolls the record back to `previous` if its staged commit is still the
/// one at `committed_revision`. Returns the deferred commit staged in its
/// place, as `publish_persisted_revision` does.
#[must_use]
pub(super) fn restore_if_revision(
    cell: &Arc<SessionCell>,
    committed_revision: u64,
    previous: SessionRollbackState,
) -> Option<AgentHistoryFollowUp> {
    let follow_up = cell.lock().ok().and_then(|mut record| {
        if record.status.revision == committed_revision {
            record.restore_rollback_state(previous);
        }
        record.stage_deferred(cell)
    });
    cell.notify_persisted();
    follow_up
}

fn publish_fork_base(
    cell: &Arc<SessionCell>,
    base: AgentHistoryBase,
) -> Result<Option<AgentHistoryFollowUp>, AgentError> {
    let mut record = cell.lock()?;
    record.conversation.history_base = Some(base);
    record.status.persisting_revision = None;
    let follow_up = record.stage_deferred(cell);
    drop(record);
    cell.notify_persisted();
    Ok(follow_up)
}

pub(super) fn append_history(record: &mut SessionRecord, items: &[ConversationItem]) {
    for item in items {
        record.conversation.history_bytes = record
            .conversation
            .history_bytes
            .saturating_add(item.retained_bytes());
        append_history_item(record, item.clone());
    }
}

pub(super) fn append_history_item(
    record: &mut SessionRecord,
    content: ConversationItem,
) -> AgentHistoryItem {
    let item_id = AgentHistoryItemId::new(record.conversation.next_history_item_id);
    record.conversation.next_history_item_id = record
        .conversation
        .next_history_item_id
        .checked_add(1)
        .expect("Agent history item id exhausted");
    let turn_id = match &content {
        ConversationItem::TurnStart { turn_id } => Some(*turn_id),
        _ => record.runtime.turn.as_ref().map(|turn| turn.id),
    };
    let item = AgentHistoryItem {
        source_session_id: record.session_id,
        item_id,
        item_revision: 0,
        turn_id,
        logical_segment: 0,
        logical_order: 0,
        turn_working_millis: None,
        content,
    };
    mutable_history(record).push(Arc::new(item.clone()));
    record
        .conversation
        .pending_history_mutations
        .push(AgentHistoryMutation::AppendItem { item: item.clone() });
    item
}

/// Settle exactly one terminal turn onto its final durable display item.
///
/// The history item is the persistence authority for completed-loop timing:
/// it survives restart and fork without carrying elapsed time into a later
/// turn. The mutation is committed atomically with the terminal runtime state.
pub(super) fn settle_turn_working_time(
    record: &mut SessionRecord,
    identity: TurnIdentity,
) -> Result<(), AgentError> {
    let turn = exact_turn(record, identity.turn_id, identity.generation)?;
    if !turn.phase.is_terminal() {
        return Err(AgentError::Protocol(
            "cannot settle working time for a non-terminal Agent turn".to_string(),
        ));
    }
    let elapsed = u64::try_from(turn.updated_at.saturating_sub(turn.created_at)).map_err(|_| {
        AgentError::Persistence("Agent turn working time exceeds the durable u64 range".to_string())
    })?;
    let durable = {
        let durable = mutable_history(record)
            .iter_mut()
            .rev()
            .find(|item| item.turn_id == Some(identity.turn_id))
            .ok_or_else(|| {
                AgentError::Persistence(
                    "terminal Agent turn has no durable display history item".to_string(),
                )
            })?;
        let durable = Arc::make_mut(durable);
        durable.item_revision = durable
            .item_revision
            .checked_add(1)
            .expect("Agent history item revision exhausted");
        durable.turn_working_millis = Some(elapsed);
        durable.clone()
    };
    record
        .conversation
        .pending_history_mutations
        .push(AgentHistoryMutation::UpsertItemSnapshot { item: durable });
    Ok(())
}

pub(super) fn persist_history_item_snapshot(
    record: &mut SessionRecord,
    model_history_index: usize,
) -> Result<(), AgentError> {
    let durable = {
        let durable = mutable_history(record)
            .get_mut(model_history_index)
            .ok_or_else(|| {
                AgentError::Persistence("Agent canonical history item is missing".to_string())
            })?;
        let durable = Arc::make_mut(durable);
        durable.item_revision = durable
            .item_revision
            .checked_add(1)
            .expect("Agent history item revision exhausted");
        durable.clone()
    };
    record
        .conversation
        .pending_history_mutations
        .push(AgentHistoryMutation::UpsertItemSnapshot { item: durable });
    Ok(())
}

pub(super) fn history_store_error(error: AgentHistoryStoreError) -> AgentError {
    AgentError::Persistence(error.to_string())
}

pub(super) fn conversation_commit(
    session_id: AgentSessionId,
    record: &mut SessionRecord,
) -> AgentConversationCommit {
    debug_assert!(record.status.persisting_revision.is_none());
    let (compression_update, compression_head) = crate::model::next_link(
        record.runtime.compression_head.as_ref(),
        record.runtime.compression.as_ref(),
    );
    record.runtime.compression_head = compression_head;
    let commit = AgentConversationCommit {
        runtime: PersistedAgentRuntimeState {
            id: session_id,
            forked_from_session_id: record.forked_from_session_id,
            parent_session_id: record.lineage.parent_session_id,
            origin: record.lineage.origin,
            cwd: record.lineage.workspace.cwd.clone(),
            branch: record.lineage.workspace.branch.clone(),
            title: record.settings.title.clone(),
            goal: record.settings.goal.clone(),
            generation: record.status.generation,
            revision: AgentRevision::new(record.status.revision),
            config: record.settings.config.clone(),
            memory_contribution: record.settings.memory_contribution,
            turn_budget_policy: record.settings.budget_policy,
            lifecycle: record.status.lifecycle,
            usage: record.runtime.usage,
            context_window: record.runtime.context_window,
            compaction: record
                .runtime
                .compaction
                .as_ref()
                .map(|compaction| compaction.snapshot.clone()),
            compression: None,
            compression_update,
            turn: record.runtime.turn.as_ref().map(|turn| PersistedAgentTurn {
                id: turn.id,
                generation: turn.generation,
                phase: turn.phase,
                sampling_step: turn.sampling_step,
                reasoning: turn.reasoning,
                final_answer_message_id: turn.final_answer_message_id,
                last_error: turn.last_error.clone(),
                terminal_reason: turn.terminal_reason,
                created_at: turn.created_at,
                updated_at: turn.updated_at,
                recovery: turn.recovery,
                budget_ledger: turn.budget_ledger,
                no_progress: turn.no_progress,
                pending_steering: turn.pending_steering.iter().cloned().collect(),
                next_steering_sequence: turn.next_steering_sequence,
                last_consumed_steering_sequence: turn.last_consumed_steering_sequence,
                model_session_generation: turn.model_session_generation,
                permission_mode: turn.permission_mode,
            }),
            wake_queue: record.runtime.wake_queue.clone(),
            activities: record.runtime.activities.persisted_snapshots(),
            task_plan: record.runtime.task_plan.clone(),
            pending_user_input: record.runtime.pending_user_input.clone(),
            next_message_id: record.conversation.next_message_id,
            next_history_item_id: record.conversation.next_history_item_id,
            model_context_boundary: record.conversation.model_context_boundary.clone(),
            history_base: record.conversation.history_base,
            created_at: record.created_at,
            updated_at: record.status.updated_at,
        },
        history_mutations: std::mem::take(&mut record.conversation.pending_history_mutations),
    };
    record.status.persisting_revision = Some(record.status.revision);
    commit
}

pub(super) fn conversation_delta(
    session_id: AgentSessionId,
    record: &mut SessionRecord,
) -> AgentConversationDelta {
    debug_assert!(record.status.persisting_revision.is_none());
    let delta = AgentConversationDelta {
        session_id,
        session_revision: AgentRevision::new(record.status.revision),
        session_updated_at: record.status.updated_at,
        history_mutations: std::mem::take(&mut record.conversation.pending_history_mutations),
    };
    record.status.persisting_revision = Some(record.status.revision);
    delta
}
