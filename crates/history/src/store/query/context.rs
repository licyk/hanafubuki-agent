use crate::store::*;

impl FileAgentHistoryStore {
    pub(in crate::store) fn load_latest_model_context_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError> {
        let runtime = self.load_runtime_state_inner(session_id)?;
        let visible = self.read_visible_items(session_id, Some(runtime.revision))?;
        let mut context = Vec::new();
        let mut segment: Option<(AgentSessionId, AgentModelContextBoundary)> = None;
        let mut items: Vec<AgentHistoryItem> = Vec::new();
        for (source_session_id, _, item) in visible {
            if segment.as_ref().map(|(id, _)| *id) != Some(source_session_id) {
                if let Some((_, boundary)) = segment.take() {
                    context.extend(segment_model_context(boundary, items));
                    items = Vec::new();
                }
                segment = Some((source_session_id, self.segment_boundary(source_session_id)?));
            }
            items.push(item);
        }
        if let Some((_, boundary)) = segment {
            context.extend(segment_model_context(boundary, items));
        }
        Ok(context)
    }

    /// The boundary `source_session_id` last persisted; `Full` for a session
    /// whose row is gone.
    fn segment_boundary(
        &self,
        source_session_id: AgentSessionId,
    ) -> Result<AgentModelContextBoundary, AgentHistoryStoreError> {
        let encoded = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            connection
                .query_row(
                    "SELECT runtime_json FROM agent_sessions WHERE session_id = ?",
                    params![to_i64(source_session_id.value(), "session id")?],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(database_error)?
        };
        let Some(encoded) = encoded else {
            return Ok(AgentModelContextBoundary::Full);
        };
        let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
            serde_json::from_str(&encoded).map_err(json_error)?;
        Ok(runtime.model_context_boundary)
    }
}

/// `items`, the lineage segment one session contributed, as that session's
/// own model context: a compacted segment is the window its compaction kept,
/// which is what its forks inherit, not its transcript.
fn segment_model_context(
    boundary: AgentModelContextBoundary,
    items: Vec<AgentHistoryItem>,
) -> Vec<AgentHistoryItem> {
    let AgentModelContextBoundary::Compacted { checkpoint } = boundary else {
        return items;
    };
    let mut context = checkpoint.replacement_history;
    context.extend(items.into_iter().filter(|item| {
        item.item_id.value() > checkpoint.future_items_after_item_id
            && item.item_id != checkpoint.checkpoint_item_id
    }));
    context
}
