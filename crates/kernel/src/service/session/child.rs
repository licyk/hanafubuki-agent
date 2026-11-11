use super::*;
use crate::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentSessionOrigin, AgentSessionPersistence,
};

/// How long closing an ephemeral session waits for what it ran to settle.
const EPHEMERAL_CLOSE_DEADLINE: Duration = Duration::from_secs(5);

/// The history a child starts from.
struct ChildHistory {
    history: Arc<Vec<Arc<AgentHistoryItem>>>,
    history_bytes: usize,
    model_context_boundary: AgentModelContextBoundary,
    next_history_item_id: u64,
    next_message_id: u64,
    context_window: AgentContextWindowSnapshot,
    /// The parent's running turn the history stops before, if one ran.
    before_turn_id: Option<AgentTurnId>,
    /// The hierarchical compression state the history comes with.
    compression: Option<Arc<crate::AgentCompressionState>>,
}

impl AgentService {
    /// Creates a child of the loaded session `request.parent_session_id`:
    /// its model, configuration and workspace are the parent's (its scope
    /// the parent's narrowed by `request.scope`), its lineage
    /// names the parent and `request.origin`, and `request.context` is
    /// committed with its creation. An ephemeral child lives in memory only
    /// until [`Self::close_ephemeral_session`] or shutdown.
    pub fn create_child_session(
        &self,
        request: AgentChildSessionRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        validate_child_context(&request.context)?;
        let parent_id = request.parent_session_id;
        let parent = (*self.session(parent_id)?.lock_after_persistence_blocking()?).clone();
        let base = self.child_history(&parent, request.base)?;
        // History starts with a turn: context can only follow one.
        if base.history.is_empty() && !request.context.is_empty() {
            return Err(AgentError::InvalidInput(
                "a child session's context follows the history it inherits, and there is none: the parent has had no turn yet"
                    .to_string(),
            ));
        }
        let session_id = self
            .inner
            .history_store
            .allocate_session_id()
            .map_err(history_store_error)?;
        let ephemeral = request.persistence == AgentSessionPersistence::Ephemeral;
        let before_turn_id = base.before_turn_id;
        let mut record = child_record(session_id, &parent, &request, base);
        normalize_session_config(&mut record.settings.config);
        validate_history_append(&record, &request.context, self.inner.limits)?;
        append_history(&mut record, &request.context);
        if ephemeral {
            self.inner
                .history_store
                .admit_ephemeral(session_id, Some(parent_id));
        }
        let cell = match self.inner.sessions.insert(record, &self.inner.accepting) {
            Ok(cell) => cell,
            Err(error) => {
                self.inner.history_store.forget_ephemeral(session_id);
                return Err(error);
            }
        };
        let persisted = match (ephemeral, request.base, before_turn_id) {
            (true, _, _) | (false, AgentChildSessionBase::Empty, _) => {
                self.persist_current_conversation(&cell, true)
            }
            (false, AgentChildSessionBase::ParentHead, None) => self.persist_current_head_fork(
                &cell,
                parent_id,
                AgentRevision::new(parent.status.revision),
            ),
            (false, AgentChildSessionBase::ParentHead, Some(turn_id)) => {
                self.persist_current_fork(&cell, parent_id, turn_id)
            }
        };
        if let Err(error) = persisted {
            self.remove_unpersisted_session(&cell);
            self.inner.history_store.forget_ephemeral(session_id);
            return Err(error);
        }
        self.snapshot(session_id)
    }

    /// Closes the ephemeral session `session_id`: cancels its running turn
    /// or compaction, waits a few seconds for it to settle, and lets go of
    /// the session and everything held of it. Off the async runtime only.
    pub fn close_ephemeral_session(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        if !self.inner.history_store.is_ephemeral(session_id) {
            return Err(AgentError::InvalidInput(format!(
                "Agent session {session_id} is not ephemeral"
            )));
        }
        let cell = self.session(session_id)?;
        let cancelled = {
            let mut record = cell.lock()?;
            if let Some(compaction) = &record.runtime.compaction {
                compaction.cancellation.cancel();
            }
            record.runtime.background_compaction.cancel();
            record
                .runtime
                .turn
                .as_ref()
                .filter(|turn| !turn.phase.is_terminal())
                .map(|turn| {
                    turn.cancellation.cancel();
                    TurnIdentity {
                        session_id,
                        turn_id: turn.id,
                        generation: turn.generation,
                    }
                })
        };
        if let Some(identity) = cancelled {
            self.abort_pending_approvals(identity);
        }
        // What has not settled by then fails on the missing session.
        let _ = cell.wait_until_blocking(Instant::now() + EPHEMERAL_CLOSE_DEADLINE, |record| {
            Ok(
                (!has_active_operation(record) && record.status.persisting_revision.is_none())
                    .then_some(()),
            )
        });
        self.await_cancelled_background_compaction(session_id);
        if let Ok(record) = cell.lock() {
            self.inner.sessions.remove(record);
        }
        self.inner.event_sinks.lock().remove(&session_id);
        self.inner.history_store.forget_ephemeral(session_id);
        Ok(())
    }

    /// The live ephemeral children of `parent_session_id`, oldest first;
    /// only those `origin` made when it is given.
    pub fn ephemeral_children(
        &self,
        parent_session_id: AgentSessionId,
        origin: Option<AgentSessionOrigin>,
    ) -> Vec<AgentSessionId> {
        let mut children = self
            .inner
            .sessions
            .cells()
            .into_iter()
            .filter(|cell| self.inner.history_store.is_ephemeral(cell.session_id()))
            .filter(|cell| {
                cell.lock().is_ok_and(|record| {
                    record.lineage.parent_session_id == Some(parent_session_id)
                        && origin.is_none_or(|origin| record.lineage.origin == Some(origin))
                })
            })
            .map(|cell| cell.session_id())
            .collect::<Vec<_>>();
        children.sort();
        children
    }

    fn child_history(
        &self,
        parent: &SessionRecord,
        base: AgentChildSessionBase,
    ) -> Result<ChildHistory, AgentError> {
        if base == AgentChildSessionBase::Empty {
            return Ok(ChildHistory {
                history: Arc::new(Vec::new()),
                history_bytes: 0,
                model_context_boundary: AgentModelContextBoundary::Full,
                next_history_item_id: 1,
                next_message_id: 1,
                context_window: AgentContextWindowSnapshot::initial(),
                before_turn_id: None,
                compression: None,
            });
        }
        let running_turn = parent
            .runtime
            .turn
            .as_ref()
            .filter(|turn| !turn.phase.is_terminal())
            .map(|turn| turn.id);
        let Some(turn_id) = running_turn else {
            ensure_head_fork_idle(parent)?;
            return Ok(ChildHistory {
                history: parent.conversation.history.clone(),
                history_bytes: parent.conversation.history_bytes,
                model_context_boundary: parent.conversation.model_context_boundary.clone(),
                next_history_item_id: parent.conversation.next_history_item_id,
                next_message_id: parent.conversation.next_message_id,
                context_window: parent.runtime.context_window,
                before_turn_id: None,
                // At the head the child has the parent's window, so the
                // same blocks project it.
                compression: parent.runtime.compression.clone(),
            });
        };
        let fork = self.load_fork_material(parent, turn_id)?;
        let contents = history_contents(&fork.history);
        validate_persisted_history(&contents, &parent.settings.config, self.inner.limits)?;
        validate_complete_tool_pairs(&contents)?;
        let history: Arc<Vec<_>> = Arc::new(fork.history.into_iter().map(Arc::new).collect());
        Ok(ChildHistory {
            history_bytes: contents.iter().fold(0usize, |total, item| {
                total.saturating_add(item.retained_bytes())
            }),
            compression: super::super::compression::inherited_state(
                parent,
                parent.runtime.compression.as_deref(),
                &history,
            ),
            history,
            model_context_boundary: AgentModelContextBoundary::Full,
            next_history_item_id: parent.conversation.next_history_item_id,
            next_message_id: parent.conversation.next_message_id,
            context_window: AgentContextWindowSnapshot::initial(),
            before_turn_id: Some(turn_id),
        })
    }
}

fn child_record(
    session_id: AgentSessionId,
    parent: &SessionRecord,
    request: &AgentChildSessionRequest,
    base: ChildHistory,
) -> SessionRecord {
    let now = now_millis();
    let from_parent = request.base == AgentChildSessionBase::ParentHead;
    SessionRecord {
        session_id,
        forked_from_session_id: from_parent.then_some(request.parent_session_id),
        lineage: SessionLineage {
            parent_session_id: Some(request.parent_session_id),
            origin: Some(request.origin),
            workspace: parent.lineage.workspace.clone(),
            ephemeral: request.persistence == AgentSessionPersistence::Ephemeral,
        },
        created_at: now,
        settings: SessionSettings {
            model: parent.settings.model.clone(),
            title: from_parent.then(|| parent.settings.title.clone()).flatten(),
            // A child works for its parent: it never pursues the parent's goal.
            goal: None,
            config: AgentSessionConfig {
                // An ephemeral session (`/side`) is too short-lived to reduce
                // its context in any other way.
                compaction_strategy: if request.persistence == AgentSessionPersistence::Ephemeral {
                    crate::AgentCompactionStrategy::Standard
                } else {
                    parent.settings.config.compaction_strategy
                },
                scope: parent.settings.config.scope.narrow(&request.scope),
                ..parent.settings.config.clone()
            },
            memory_contribution: AgentMemoryContributionState::default(),
            effective_input_capabilities: parent.settings.effective_input_capabilities,
            budget_policy: parent.settings.budget_policy,
        },
        status: SessionStatus::created(now),
        conversation: SessionConversation {
            history: base.history,
            history_version: 0,
            pending_history_mutations: Vec::new(),
            next_history_item_id: base.next_history_item_id,
            model_context_boundary: base.model_context_boundary,
            history_base: None,
            history_bytes: base.history_bytes,
            next_message_id: base.next_message_id,
        },
        journal: SessionJournal::empty(),
        runtime: SessionRuntime {
            context_window: base.context_window,
            compression_runtime: parent
                .runtime
                .compression_runtime
                .forked(base.compression.as_deref()),
            compression: base.compression,
            ..SessionRuntime::idle()
        },
    }
}

fn validate_child_context(context: &[ConversationItem]) -> Result<(), AgentError> {
    if context
        .iter()
        .all(|item| matches!(item, ConversationItem::ContinuationContext { .. }))
    {
        Ok(())
    } else {
        Err(AgentError::InvalidInput(
            "a child session's context is continuation context only".to_string(),
        ))
    }
}
