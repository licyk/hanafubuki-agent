use super::*;

impl AgentService {
    pub fn fork_session_before_turn(
        &self,
        request: AgentSessionForkRequest,
    ) -> Result<AgentSessionForkOutcome, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let source_cell = self.session(request.source_session_id)?;
        let source = {
            let record = source_cell.lock_after_persistence_blocking()?;
            if !self.inner.accepting.load(Ordering::Acquire) {
                return Err(AgentError::ShuttingDown);
            }
            (*record).clone()
        };
        validate_session_mutation(&source, request.source_generation, request.source_revision)?;
        let fork = self.load_fork_material(&source, request.before_turn_id)?;
        let history_contents = history_contents(&fork.history);
        validate_persisted_history(
            &history_contents,
            &source.settings.config,
            self.inner.limits,
        )?;
        validate_complete_tool_pairs(&history_contents)?;

        let session_id = self
            .inner
            .history_store
            .allocate_session_id()
            .map_err(history_store_error)?;
        let source = (*source_cell.lock_after_persistence_blocking()?).clone();
        validate_session_mutation(&source, request.source_generation, request.source_revision)?;
        let now = now_millis();
        let history_bytes = history_contents.iter().fold(0usize, |total, item| {
            total.saturating_add(item.retained_bytes())
        });
        let record = SessionRecord {
            session_id,
            forked_from_session_id: Some(request.source_session_id),
            lineage: SessionLineage {
                workspace: source.lineage.workspace.clone(),
                ..SessionLineage::default()
            },
            // A fork inherits history but not the source's execution clock: the
            // turns it kept were executed by a different session, and the ones
            // it dropped must not be counted at all.
            created_at: now,
            settings: SessionSettings {
                model: source.settings.model.clone(),
                title: source.settings.title.clone(),
                goal: source.settings.goal.clone(),
                config: source.settings.config.clone(),
                memory_contribution: AgentMemoryContributionState::default(),
                effective_input_capabilities: source.settings.effective_input_capabilities,
                budget_policy: source.settings.budget_policy,
            },
            status: SessionStatus::created(now),
            conversation: SessionConversation {
                history: Arc::new(fork.history.into_iter().map(Arc::new).collect()),
                history_version: 0,
                pending_history_mutations: Vec::new(),
                next_history_item_id: source.conversation.next_history_item_id,
                model_context_boundary: AgentModelContextBoundary::Full,
                history_base: None,
                history_bytes,
                next_message_id: source.conversation.next_message_id,
            },
            journal: SessionJournal::empty(),
            runtime: SessionRuntime::idle(),
        };
        let cell = self.inner.sessions.insert(record, &self.inner.accepting)?;
        if let Err(error) =
            self.persist_current_fork(&cell, request.source_session_id, request.before_turn_id)
        {
            self.remove_unpersisted_session(&cell);
            return Err(error);
        }
        Ok(AgentSessionForkOutcome {
            source_session_id: request.source_session_id,
            before_turn_id: request.before_turn_id,
            restored_prompt: fork.restored_prompt,
            restored_images: fork.restored_images,
            session: self.snapshot(session_id)?,
        })
    }

    pub fn fork_session_at_head(
        &self,
        request: AgentSessionHeadForkRequest,
    ) -> Result<AgentSessionHeadForkOutcome, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let source_cell = self.session(request.source_session_id)?;
        {
            let source = source_cell.lock_after_persistence_blocking()?;
            validate_session_mutation(&source, request.source_generation, request.source_revision)?;
            ensure_no_staged_commit(&source)?;
            ensure_head_fork_idle(&source)?;
        }
        let session_id = self
            .inner
            .history_store
            .allocate_session_id()
            .map_err(history_store_error)?;
        let source = (*source_cell.lock_after_persistence_blocking()?).clone();
        validate_session_mutation(&source, request.source_generation, request.source_revision)?;
        ensure_no_staged_commit(&source)?;
        ensure_head_fork_idle(&source)?;
        let now = now_millis();
        let record = SessionRecord {
            session_id,
            forked_from_session_id: Some(request.source_session_id),
            lineage: SessionLineage {
                workspace: source.lineage.workspace.clone(),
                ..SessionLineage::default()
            },
            created_at: now,
            settings: SessionSettings {
                model: source.settings.model.clone(),
                title: source.settings.title.clone(),
                goal: source.settings.goal.clone(),
                config: source.settings.config.clone(),
                memory_contribution: AgentMemoryContributionState::default(),
                effective_input_capabilities: source.settings.effective_input_capabilities,
                budget_policy: source.settings.budget_policy,
            },
            status: SessionStatus::created(now),
            conversation: SessionConversation {
                history: source.conversation.history.clone(),
                history_version: source.conversation.history_version,
                pending_history_mutations: Vec::new(),
                next_history_item_id: source.conversation.next_history_item_id,
                model_context_boundary: source.conversation.model_context_boundary.clone(),
                history_base: None,
                history_bytes: source.conversation.history_bytes,
                next_message_id: source.conversation.next_message_id,
            },
            journal: SessionJournal::empty(),
            runtime: SessionRuntime {
                turn: None,
                compaction: None,
                usage: None,
                context_window: source.runtime.context_window,
                activities: AgentActivityTracker::default(),
                task_plan: None,
                pending_user_input: None,
                allowed_tools: BTreeSet::new(),
                permission_mode: None,
                sandbox_mode: None,
            },
        };
        let cell = self.inner.sessions.insert(record, &self.inner.accepting)?;
        if let Err(error) = self.persist_current_head_fork(
            &cell,
            request.source_session_id,
            request.source_revision,
        ) {
            self.remove_unpersisted_session(&cell);
            return Err(error);
        }
        Ok(AgentSessionHeadForkOutcome {
            source_session_id: request.source_session_id,
            source_revision: request.source_revision,
            session: self.snapshot(session_id)?,
        })
    }

    pub(super) fn load_fork_material(
        &self,
        source: &SessionRecord,
        before_turn_id: AgentTurnId,
    ) -> Result<ForkMaterial, AgentError> {
        let mut scan = ForkScan::new(
            source,
            before_turn_id,
            self.inner.limits.maximum_history_items,
        );
        scan.extend(
            source
                .conversation
                .history
                .iter()
                .map(|item| item.as_ref().clone()),
        );
        if let Some(material) = scan.finish_if_found()? {
            return Ok(material);
        }

        let mut scan = ForkScan::new(
            source,
            before_turn_id,
            self.inner.limits.maximum_history_items,
        );
        let mut cursor = None;
        loop {
            let page = self
                .inner
                .history_store
                .list_items_page(AgentTranscriptPageRequest {
                    session_id: source.session_id,
                    page_size: crate::AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
                    cursor,
                    direction: AgentHistorySortDirection::Ascending,
                    through_revision: Some(AgentRevision::new(source.status.revision)),
                })
                .map_err(history_store_error)?;
            scan.extend(page.items);
            if let Some(material) = scan.finish_if_found()? {
                return Ok(material);
            }
            let Some(next_cursor) = page.newer_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
        Err(AgentError::TurnNotEditable)
    }
}
