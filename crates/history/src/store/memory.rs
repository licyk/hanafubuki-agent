use std::collections::HashMap;

use crate::store::jsonl::{read_complete_records, validate_record};
use crate::store::*;

const MEMORY_CANDIDATE_LIMIT_MAX: usize = 100;
const MEMORY_EVIDENCE_ITEMS_MAX: usize = 2_000;
const MEMORY_EVIDENCE_CHARS_MAX: usize = 2_000_000;

impl FileAgentHistoryStore {
    pub(in crate::store) fn list_memory_candidates_inner(
        &self,
        query: AgentMemoryHistoryQuery,
    ) -> Result<Vec<AgentMemoryHistoryCandidate>, AgentHistoryStoreError> {
        if query.limit == 0 || query.limit > MEMORY_CANDIDATE_LIMIT_MAX {
            return Err(AgentHistoryStoreError::InvalidRequest(format!(
                "memory candidate limit must be between 1 and {MEMORY_CANDIDATE_LIMIT_MAX}"
            )));
        }
        let idle_before = query
            .now_millis
            .saturating_sub(u128::from(query.minimum_idle_millis));
        let age_after = query
            .now_millis
            .saturating_sub(u128::from(query.maximum_age_millis));
        let current_session_id = query
            .current_session_id
            .map(|session_id| to_i64(session_id.value(), "current session id"))
            .transpose()?;
        let (older_updated_at, older_session_id) = match query.older_than {
            Some(candidate) => (
                Some(millis_to_i64(candidate.updated_at)?),
                Some(to_i64(candidate.session_id.value(), "session id")?),
            ),
            None => (None, None),
        };
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                r#"
SELECT sessions.session_id, sessions.revision, sessions.updated_at
FROM agent_sessions AS sessions
JOIN agent_history_projection_state AS projection
  ON projection.session_id = sessions.session_id
WHERE sessions.lifecycle = 'available'
  AND sessions.latest_turn_phase IN ('completed', 'cancelled', 'failed', 'interrupted')
  AND projection.last_projected_session_revision = sessions.revision
  AND CAST(sessions.updated_at AS INTEGER) <= ?1
  AND CAST(sessions.updated_at AS INTEGER) >= ?2
  AND (?3 IS NULL OR sessions.session_id != ?3)
  AND (?5 IS NULL OR CAST(sessions.updated_at AS INTEGER) < ?5
       OR (CAST(sessions.updated_at AS INTEGER) = ?5 AND sessions.session_id < ?6))
ORDER BY CAST(sessions.updated_at AS INTEGER) DESC, sessions.session_id DESC
LIMIT ?4
                "#,
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(
                params![
                    millis_to_i64(idle_before)?,
                    millis_to_i64(age_after)?,
                    current_session_id,
                    to_i64(query.limit as u64, "memory candidate limit")?,
                    older_updated_at,
                    older_session_id,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(database_error)?;
        let mut candidates = Vec::new();
        for row in rows {
            let (session_id, revision, updated_at) = row.map_err(database_error)?;
            let session_id = AgentSessionId::new(from_i64(session_id, "session id")?);
            if self.inner.projection_lag.lock().contains_key(&session_id) {
                continue;
            }
            candidates.push(AgentMemoryHistoryCandidate {
                session_id,
                source_revision: AgentRevision::new(from_i64(revision, "session revision")?),
                updated_at: updated_at.parse::<u128>().map_err(|_| {
                    AgentHistoryStoreError::CorruptHistory(
                        "session updated_at is not a millisecond timestamp".to_string(),
                    )
                })?,
            });
        }
        Ok(candidates)
    }

    pub(in crate::store) fn load_memory_evidence_inner(
        &self,
        candidate: AgentMemoryHistoryCandidate,
        maximum_items: usize,
        maximum_chars: usize,
    ) -> Result<AgentMemoryEvidence, AgentHistoryStoreError> {
        if maximum_items == 0 || maximum_items > MEMORY_EVIDENCE_ITEMS_MAX {
            return Err(AgentHistoryStoreError::InvalidRequest(format!(
                "memory evidence item limit must be between 1 and {MEMORY_EVIDENCE_ITEMS_MAX}"
            )));
        }
        if maximum_chars == 0 || maximum_chars > MEMORY_EVIDENCE_CHARS_MAX {
            return Err(AgentHistoryStoreError::InvalidRequest(format!(
                "memory evidence character limit must be between 1 and {MEMORY_EVIDENCE_CHARS_MAX}"
            )));
        }

        if let Some(writer) = self
            .inner
            .writers
            .lock()
            .get(&candidate.session_id)
            .cloned()
        {
            let (ack_tx, ack_rx) = mpsc::sync_channel(1);
            writer
                .sender
                .send(super::writer::WriterCommand::Flush { ack: ack_tx })
                .map_err(|_| AgentHistoryStoreError::ShuttingDown)?;
            ack_rx
                .recv()
                .map_err(|_| AgentHistoryStoreError::ShuttingDown)??;
        }

        let path = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            connection
                .query_row(
                    r#"
SELECT sessions.rollout_path
FROM agent_sessions AS sessions
JOIN agent_history_projection_state AS projection
  ON projection.session_id = sessions.session_id
WHERE sessions.session_id = ?1
  AND sessions.lifecycle = 'available'
  AND sessions.revision = ?2
  AND projection.last_projected_session_revision = sessions.revision
                    "#,
                    params![
                        to_i64(candidate.session_id.value(), "session id")?,
                        to_i64(candidate.source_revision.value(), "session revision")?,
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(database_error)?
                .ok_or(AgentHistoryStoreError::StaleRevision)?
        };
        let path = PathBuf::from(path);
        ensure_scoped_path(&path, &self.inner.paths.root)?;
        let records = read_complete_records(&path, 0)?;
        let mut next_ordinal = 0;
        let mut latest_revision = None;
        let mut item_order = Vec::new();
        let mut items = HashMap::new();
        for complete in records.records {
            validate_record(candidate.session_id, next_ordinal, &complete.record)?;
            next_ordinal = next_ordinal.checked_add(1).ok_or_else(|| {
                AgentHistoryStoreError::CorruptHistory("record ordinal exhausted".to_string())
            })?;
            latest_revision = Some(complete.record.session_revision);
            for mutation in complete.record.payload.history_mutations {
                let item = match mutation {
                    AgentHistoryMutation::AppendItem { item }
                    | AgentHistoryMutation::UpsertItemSnapshot { item } => item,
                    AgentHistoryMutation::SetModelContextBoundary { .. } => continue,
                };
                if item.source_session_id != candidate.session_id {
                    continue;
                }
                if !items.contains_key(&item.item_id) {
                    item_order.push(item.item_id);
                }
                items.insert(item.item_id, item);
            }
        }
        if latest_revision != Some(candidate.source_revision) {
            return Err(AgentHistoryStoreError::StaleRevision);
        }

        let mut evidence = Vec::new();
        let mut used_chars = 0usize;
        let mut truncated = false;
        for item_id in item_order {
            let Some(item) = items.remove(&item_id) else {
                continue;
            };
            let Some((role, text, tool_name, tool_arguments, call_id)) =
                memory_evidence_text(&item.content)
            else {
                continue;
            };
            let text = text.trim();
            if text.is_empty() && tool_name.is_none() {
                continue;
            }
            if evidence.len() == maximum_items {
                truncated = true;
                break;
            }
            let remaining = maximum_chars.saturating_sub(used_chars);
            if remaining == 0 {
                truncated = true;
                break;
            }
            let (text, text_truncated) = bounded_chars(text, remaining);
            used_chars = used_chars.saturating_add(text.chars().count());
            evidence.push(AgentMemoryEvidenceItem {
                item_id,
                turn_id: item.turn_id,
                role,
                text,
                tool_name,
                tool_arguments,
                call_id,
            });
            if text_truncated {
                truncated = true;
                break;
            }
        }
        Ok(AgentMemoryEvidence {
            session_id: candidate.session_id,
            source_revision: candidate.source_revision,
            updated_at: candidate.updated_at,
            items: evidence,
            truncated,
        })
    }
}

/// An evidence item's role, text, tool name, tool arguments and call id.
type EvidenceText = (
    AgentMemoryEvidenceRole,
    String,
    Option<String>,
    Option<serde_json::Value>,
    Option<String>,
);

fn memory_evidence_text(item: &ConversationItem) -> Option<EvidenceText> {
    match item {
        ConversationItem::UserMessage { text, .. } => Some((
            AgentMemoryEvidenceRole::User,
            text.clone(),
            None,
            None,
            None,
        )),
        ConversationItem::AssistantMessage { text, .. } => Some((
            AgentMemoryEvidenceRole::Assistant,
            text.clone(),
            None,
            None,
            None,
        )),
        ConversationItem::SteeringMessage { text, .. } => Some((
            AgentMemoryEvidenceRole::Steering,
            text.clone(),
            None,
            None,
            None,
        )),
        ConversationItem::ToolCall {
            call_id,
            namespace,
            name,
            arguments,
            ..
        } => Some((
            AgentMemoryEvidenceRole::Tool,
            String::new(),
            Some(
                hanafubuki_agent_kernel::flattened_tool_call_name(namespace.as_deref(), name)
                    .into_owned(),
            ),
            Some(arguments.clone()),
            Some(call_id.clone()),
        )),
        ConversationItem::ToolOutput {
            call_id, output, ..
        } => output.legacy_text_projection().ok().map(|text| {
            (
                AgentMemoryEvidenceRole::Tool,
                text,
                None,
                None,
                Some(call_id.clone()),
            )
        }),
        // The command the person ran, as a call of a tool of that name, and
        // what it printed as its output.
        ConversationItem::UserShellCommand {
            call_id,
            command,
            output,
            ..
        } => Some((
            AgentMemoryEvidenceRole::Tool,
            output.clone(),
            Some("user_shell_command".to_string()),
            Some(serde_json::json!({ "cmd": command })),
            Some(call_id.clone()),
        )),
        // What background work reported, as the output of a call of that
        // name: data, never the person's words.
        ConversationItem::WakeEvents { events } => Some((
            AgentMemoryEvidenceRole::Tool,
            hanafubuki_agent_kernel::wake_events_record(events),
            Some("background_events".to_string()),
            None,
            None,
        )),
        ConversationItem::CanonicalInitialContext { .. }
        | ConversationItem::Reasoning { .. }
        | ConversationItem::TurnStart { .. }
        | ConversationItem::WorldStateUpdate { .. }
        | ConversationItem::SkillContext { .. }
        | ConversationItem::ToolSelection { .. }
        | ConversationItem::PermissionReview { .. }
        | ConversationItem::ContinuationContext { .. }
        | ConversationItem::CompactionSummary { .. }
        | ConversationItem::ProviderCompaction { .. } => None,
    }
}

fn bounded_chars(value: &str, maximum: usize) -> (String, bool) {
    let mut chars = value.chars();
    let bounded = chars.by_ref().take(maximum).collect::<String>();
    (bounded, chars.next().is_some())
}

fn millis_to_i64(value: u128) -> Result<i64, AgentHistoryStoreError> {
    i64::try_from(value).map_err(|_| {
        AgentHistoryStoreError::InvalidRequest("millisecond timestamp is out of range".to_string())
    })
}
