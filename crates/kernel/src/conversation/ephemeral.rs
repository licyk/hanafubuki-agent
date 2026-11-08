use super::*;

#[derive(Default)]
pub struct EphemeralAgentHistoryStore {
    next_session_id: parking_lot::Mutex<u64>,
    sessions: parking_lot::Mutex<std::collections::HashMap<AgentSessionId, AgentRecoveredSession>>,
    attachments: parking_lot::Mutex<
        std::collections::HashMap<(AgentSessionId, crate::AgentAttachmentId), Vec<u8>>,
    >,
}

impl AgentHistoryStore for EphemeralAgentHistoryStore {
    fn allocate_session_id(&self) -> Result<AgentSessionId, AgentHistoryStoreError> {
        let mut next = self.next_session_id.lock();
        if *next == 0 {
            *next = 1;
        }
        let id = AgentSessionId::new(*next);
        *next = next.checked_add(1).expect("Agent session id exhausted");
        Ok(id)
    }

    fn create_session(
        &self,
        commit: AgentConversationCommit,
    ) -> Result<(), AgentHistoryStoreError> {
        self.sessions.lock().insert(
            commit.runtime.id,
            AgentRecoveredSession {
                runtime: commit.runtime,
                model_context: appended_items(commit.history_mutations),
            },
        );
        Ok(())
    }

    fn append_commit(&self, commit: AgentConversationCommit) -> Result<(), AgentHistoryStoreError> {
        let mut sessions = self.sessions.lock();
        let session =
            sessions
                .get_mut(&commit.runtime.id)
                .ok_or(AgentHistoryStoreError::NotFound {
                    session_id: commit.runtime.id,
                })?;
        apply_memory_mutations(&mut session.model_context, &commit.history_mutations);
        session.runtime = commit.runtime;
        Ok(())
    }

    fn append_delta(&self, delta: AgentConversationDelta) -> Result<(), AgentHistoryStoreError> {
        let mut sessions = self.sessions.lock();
        let session =
            sessions
                .get_mut(&delta.session_id)
                .ok_or(AgentHistoryStoreError::NotFound {
                    session_id: delta.session_id,
                })?;
        apply_memory_mutations(&mut session.model_context, &delta.history_mutations);
        Ok(())
    }

    /// In memory, the append completes, and is acknowledged, at once, and
    /// so do its follow-ups.
    fn enqueue_commit(&self, commit: AgentConversationCommit, ack: AgentHistoryAppendAck) {
        ack.acknowledge(self.append_commit(commit), |commit| {
            self.append_commit(commit)
        });
    }

    fn enqueue_delta(&self, delta: AgentConversationDelta, ack: AgentHistoryAppendAck) {
        ack.acknowledge(self.append_delta(delta), |commit| {
            self.append_commit(commit)
        });
    }

    fn create_fork(
        &self,
        source_session_id: AgentSessionId,
        _before_turn_id: AgentTurnId,
        mut commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        let base = AgentHistoryBase {
            session_id: source_session_id,
            end_ordinal_exclusive: 0,
            end_byte_offset: 0,
        };
        commit.runtime.history_base = Some(base);
        self.create_session(commit)?;
        Ok(base)
    }

    fn create_fork_at_head(
        &self,
        source_session_id: AgentSessionId,
        _source_revision: AgentRevision,
        mut commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        if !self.sessions.lock().contains_key(&source_session_id) {
            return Err(AgentHistoryStoreError::NotFound {
                session_id: source_session_id,
            });
        }
        let base = AgentHistoryBase {
            session_id: source_session_id,
            end_ordinal_exclusive: 0,
            end_byte_offset: 0,
        };
        commit.runtime.history_base = Some(base);
        self.create_session(commit)?;
        Ok(base)
    }

    fn stage_image_attachment(
        &self,
        session_id: AgentSessionId,
        image: crate::PreparedAgentImage,
    ) -> Result<crate::AgentImageAttachmentRef, AgentHistoryStoreError> {
        if !self.sessions.lock().contains_key(&session_id) {
            return Err(AgentHistoryStoreError::NotFound { session_id });
        }
        let digest = format!("{:064x}", image.bytes.len());
        let attachment_id = crate::AgentAttachmentId::from_sha256(digest);
        let byte_length = u64::try_from(image.bytes.len()).map_err(|_| {
            AgentHistoryStoreError::Io("attachment byte length exceeds u64".to_string())
        })?;
        self.attachments
            .lock()
            .insert((session_id, attachment_id.clone()), image.bytes);
        Ok(crate::AgentImageAttachmentRef {
            attachment_id,
            source_session_id: session_id,
            mime_type: image.mime_type,
            byte_length,
            width: image.width,
            height: image.height,
            display_name: image.display_name,
            detail: image.detail,
        })
    }

    fn load_image_attachment(
        &self,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        self.attachments
            .lock()
            .get(&(
                attachment.source_session_id,
                attachment.attachment_id.clone(),
            ))
            .cloned()
            .ok_or_else(|| {
                AgentHistoryStoreError::CorruptHistory(format!(
                    "image attachment {} is missing",
                    attachment.attachment_id.as_str()
                ))
            })
    }

    fn cleanup_unreferenced_image_attachments(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        let referenced = self
            .sessions
            .lock()
            .values()
            .flat_map(|session| session.model_context.iter())
            .flat_map(|item| match &item.content {
                ConversationItem::UserMessage { images, .. }
                | ConversationItem::SteeringMessage { images, .. }
                | ConversationItem::ToolOutput { images, .. } => images.clone(),
                _ => Vec::new(),
            })
            .filter(|image| image.source_session_id == session_id)
            .map(|image| image.attachment_id)
            .collect::<std::collections::HashSet<_>>();
        self.attachments
            .lock()
            .retain(|(owner, id), _| *owner != session_id || referenced.contains(id));
        Ok(())
    }

    fn load_visible_image_attachment(
        &self,
        session_id: AgentSessionId,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        let sessions = self.sessions.lock();
        let mut current = Some(session_id);
        let mut visible = false;
        while let Some(id) = current {
            if let Some(session) = sessions.get(&id) {
                visible |= session
                    .model_context
                    .iter()
                    .any(|item| match &item.content {
                        ConversationItem::UserMessage { images, .. }
                        | ConversationItem::SteeringMessage { images, .. }
                        | ConversationItem::ToolOutput { images, .. } => {
                            images.contains(attachment)
                        }
                        _ => false,
                    });
                if visible {
                    break;
                }
            }
            current = sessions
                .get(&id)
                .and_then(|session| session.runtime.history_base.map(|base| base.session_id));
        }
        drop(sessions);
        if !visible {
            return Err(AgentHistoryStoreError::NotFound { session_id });
        }
        self.load_image_attachment(attachment)
    }

    fn flush_session(&self, _session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }

    fn shutdown_session(&self, _session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }

    fn discard_live_session(&self, _session_id: AgentSessionId) {}

    fn load_recovery_candidates(
        &self,
    ) -> Result<Vec<Result<AgentRecoveredSession, AgentRecoveryLoadFailure>>, AgentHistoryStoreError>
    {
        Ok(self.sessions.lock().values().cloned().map(Ok).collect())
    }

    fn load_runtime_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<PersistedAgentRuntimeState, AgentHistoryStoreError> {
        self.sessions
            .lock()
            .get(&session_id)
            .map(|session| session.runtime.clone())
            .ok_or(AgentHistoryStoreError::NotFound { session_id })
    }

    fn load_latest_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError> {
        self.sessions
            .lock()
            .get(&session_id)
            .map(|session| session.model_context.clone())
            .ok_or(AgentHistoryStoreError::NotFound { session_id })
    }

    fn list_sessions_page(
        &self,
        _request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentHistoryStoreError> {
        Ok(AgentSessionPage {
            sessions: Vec::new(),
            next_cursor: None,
        })
    }

    fn list_items_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        let sessions = self.sessions.lock();
        let session =
            sessions
                .get(&request.session_id)
                .ok_or(AgentHistoryStoreError::NotFound {
                    session_id: request.session_id,
                })?;
        Ok(AgentTranscriptPage {
            items: session.model_context.clone(),
            older_cursor: None,
            newer_cursor: None,
            through_revision: session.runtime.revision,
        })
    }

    fn list_turn_items_page(
        &self,
        _request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        Ok(AgentTranscriptPage {
            items: Vec::new(),
            older_cursor: None,
            newer_cursor: None,
            through_revision: AgentRevision::new(0),
        })
    }

    fn archive_session(
        &self,
        _session_id: AgentSessionId,
        _expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }

    fn restore_session(
        &self,
        _session_id: AgentSessionId,
        _expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }

    fn check_sessions_deletable(
        &self,
        _session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }

    fn delete_sessions_confirmed(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        let mut sessions = self.sessions.lock();
        for session_id in session_ids {
            sessions.remove(session_id);
        }
        self.attachments
            .lock()
            .retain(|(session_id, _), _| !session_ids.contains(session_id));
        Ok(())
    }

    fn repair_session_projection(
        &self,
        _session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }

    fn shutdown(&self, _deadline: std::time::Instant) -> Vec<AgentHistoryStoreError> {
        Vec::new()
    }
}

fn appended_items(mutations: Vec<AgentHistoryMutation>) -> Vec<AgentHistoryItem> {
    let mut items = Vec::new();
    apply_memory_mutations(&mut items, &mutations);
    items
}

fn apply_memory_mutations(items: &mut Vec<AgentHistoryItem>, mutations: &[AgentHistoryMutation]) {
    for mutation in mutations {
        match mutation {
            AgentHistoryMutation::AppendItem { item } => items.push(item.clone()),
            AgentHistoryMutation::UpsertItemSnapshot { item } => {
                if let Some(existing) = items
                    .iter_mut()
                    .find(|existing| existing.item_id == item.item_id)
                {
                    *existing = item.clone();
                } else {
                    items.push(item.clone());
                }
            }
            AgentHistoryMutation::SetModelContextBoundary { .. } => {}
        }
    }
}
