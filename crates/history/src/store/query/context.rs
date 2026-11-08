use crate::store::*;

impl FileAgentHistoryStore {
    pub(in crate::store) fn load_latest_model_context_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError> {
        let runtime = self.load_runtime_state_inner(session_id)?;
        let visible = self.read_visible_items(session_id, Some(runtime.revision))?;
        let items = visible
            .into_iter()
            .map(|(_, _, item)| item)
            .collect::<Vec<_>>();
        match runtime.model_context_boundary {
            AgentModelContextBoundary::Full => Ok(items),
            AgentModelContextBoundary::Compacted { checkpoint } => {
                let mut context = checkpoint.replacement_history;
                context.extend(items.into_iter().filter(|item| {
                    item.item_id.value() > checkpoint.future_items_after_item_id
                        && item.item_id != checkpoint.checkpoint_item_id
                }));
                Ok(context)
            }
        }
    }
}
