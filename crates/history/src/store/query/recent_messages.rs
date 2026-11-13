//! What the person sent across the top-level sessions, for an interface's
//! history search: their messages, steers and `!` commands, newest first.
//! A subagent's session is left out, its messages being its parent's
//! model's. The read is tolerant: an item that does not decode is skipped,
//! and nothing of its session is refused.

use crate::store::*;
use hanafubuki_agent_kernel::RECENT_USER_MESSAGES_MAX;

impl FileAgentHistoryStore {
    pub(in crate::store) fn recent_user_messages_inner(
        &self,
        limit: usize,
        before: Option<u128>,
    ) -> Result<Vec<String>, AgentHistoryStoreError> {
        let limit = limit.min(RECENT_USER_MESSAGES_MAX);
        let before = before
            .map(|before| {
                i64::try_from(before).map_err(|_| {
                    AgentHistoryStoreError::InvalidRequest(
                        "millisecond timestamp is out of range".to_string(),
                    )
                })
            })
            .transpose()?;
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                r#"
SELECT items.item_json
FROM agent_history_items AS items
JOIN agent_sessions AS sessions ON sessions.session_id = items.session_id
WHERE items.item_type IN ('user_message', 'steering_message', 'user_shell_command')
  AND sessions.parent_session_id IS NULL
  AND (?1 IS NULL OR CAST(items.created_at AS INTEGER) < ?1)
ORDER BY CAST(items.created_at AS INTEGER) DESC, items.session_id DESC,
         items.first_item_order DESC
                "#,
            )
            .map_err(database_error)?;
        let mut rows = statement.query(params![before]).map_err(database_error)?;
        let mut seen = HashSet::new();
        let mut messages = Vec::new();
        while messages.len() < limit {
            let Some(row) = rows.next().map_err(database_error)? else {
                break;
            };
            let encoded: String = row.get(0).map_err(database_error)?;
            let Some(text) = serde_json::from_str::<AgentHistoryItem>(&encoded)
                .ok()
                .and_then(|item| typed_text(item.content))
            else {
                continue;
            };
            if seen.insert(text.clone()) {
                messages.push(text);
            }
        }
        Ok(messages)
    }
}

/// The text as the person typed it; none for a message with no text (an
/// image alone).
fn typed_text(content: ConversationItem) -> Option<String> {
    let text = match content {
        ConversationItem::UserMessage { text, .. }
        | ConversationItem::SteeringMessage { text, .. } => text,
        ConversationItem::UserShellCommand { command, .. } => format!("!{command}"),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}
