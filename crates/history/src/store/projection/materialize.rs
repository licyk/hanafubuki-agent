use crate::store::jsonl::{read_complete_records, validate_record};
use crate::store::*;

impl FileAgentHistoryStore {
    pub(in crate::store) fn materialize_path(
        &self,
        session_id: AgentSessionId,
        path: &Path,
    ) -> Result<(), AgentHistoryStoreError> {
        ensure_scoped_path(path, &self.inner.paths.root)?;
        let _database = self.inner.database_gate.lock();
        let mut connection = self.connection()?;
        let checkpoint = connection
            .query_row(
                "SELECT next_jsonl_byte_offset, next_ordinal FROM agent_history_projection_state WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(database_error)?
            .unwrap_or((0, 0));
        let start_offset = from_i64(checkpoint.0, "projection byte offset")?;
        let expected_ordinal = from_i64(checkpoint.1, "projection ordinal")?;
        let complete = read_complete_records(path, start_offset)?;
        if complete.records.is_empty() && complete.next_offset == start_offset {
            return Ok(());
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let observed = transaction
            .query_row(
                "SELECT next_jsonl_byte_offset, next_ordinal FROM agent_history_projection_state WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(database_error)?
            .unwrap_or((0, 0));
        if observed != checkpoint {
            return Err(AgentHistoryStoreError::ProjectionUnavailable(format!(
                "session {session_id} projection changed while materializing"
            )));
        }
        let mut next_ordinal = expected_ordinal;
        let mut last_revision = 0;
        for complete_record in complete.records {
            let record = complete_record.record;
            validate_record(session_id, next_ordinal, &record)?;
            project_record(
                &transaction,
                path,
                &record,
                complete_record.start_offset,
                complete_record.end_offset,
            )?;
            next_ordinal = next_ordinal.checked_add(1).ok_or_else(|| {
                AgentHistoryStoreError::CorruptHistory("record ordinal exhausted".to_string())
            })?;
            last_revision = record.session_revision.value();
        }
        transaction
            .execute(
                r#"
INSERT INTO agent_history_projection_state(
    session_id, next_jsonl_byte_offset, next_ordinal, last_projected_session_revision
) VALUES (?, ?, ?, ?)
ON CONFLICT(session_id) DO UPDATE SET
    next_jsonl_byte_offset = excluded.next_jsonl_byte_offset,
    next_ordinal = excluded.next_ordinal,
    last_projected_session_revision = excluded.last_projected_session_revision
                "#,
                params![
                    to_i64(session_id.value(), "session id")?,
                    to_i64(complete.next_offset, "projection byte offset")?,
                    to_i64(next_ordinal, "projection ordinal")?,
                    to_i64(last_revision, "session revision")?,
                ],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)
    }
}

fn project_record(
    transaction: &rusqlite::Transaction<'_>,
    path: &Path,
    record: &AgentHistoryRecord,
    start_offset: u64,
    end_offset: u64,
) -> Result<(), AgentHistoryStoreError> {
    let runtime = record.payload.runtime_checkpoint.as_ref();
    transaction
        .execute(
            "UPDATE agent_store_state SET value = MAX(value, ?) WHERE key = 'next_session_id'",
            params![to_i64(
                record.session_id.value().checked_add(1).ok_or_else(|| {
                    AgentHistoryStoreError::CorruptHistory("session id exhausted".to_string())
                })?,
                "next session id",
            )?],
        )
        .map_err(database_error)?;
    if let Some(runtime) = runtime {
        project_compression_link(transaction, record, runtime)?;
        let runtime_json = runtime_json(runtime)?;
        let summary = session_summary(runtime);
        let summary_json = serde_json::to_string(&summary).map_err(json_error)?;
        let recovery_needed = runtime_recovery_needed(runtime);
        transaction
            .execute(
            r#"
INSERT INTO agent_sessions(
    session_id, rollout_path, created_at, updated_at, lifecycle, generation, revision, checkpoint_revision,
    model, mode, title, latest_turn_id, latest_turn_phase, recovery_needed,
    runtime_json, summary_json, forked_from_session_id, archived_at, parent_session_id
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(session_id) DO UPDATE SET
    rollout_path = excluded.rollout_path,
    updated_at = excluded.updated_at,
    lifecycle = excluded.lifecycle,
    generation = excluded.generation,
    revision = excluded.revision,
    checkpoint_revision = excluded.checkpoint_revision,
    model = excluded.model,
    mode = excluded.mode,
    title = excluded.title,
    latest_turn_id = excluded.latest_turn_id,
    latest_turn_phase = excluded.latest_turn_phase,
    recovery_needed = excluded.recovery_needed,
    runtime_json = excluded.runtime_json,
    summary_json = excluded.summary_json,
    forked_from_session_id = excluded.forked_from_session_id,
    archived_at = excluded.archived_at,
    parent_session_id = excluded.parent_session_id
            "#,
                params![
                    to_i64(runtime.id.value(), "session id")?,
                    path.to_string_lossy(),
                    runtime.created_at.to_string(),
                    runtime.updated_at.to_string(),
                    lifecycle_name(runtime.lifecycle),
                    to_i64(runtime.generation.value(), "generation")?,
                    to_i64(record.session_revision.value(), "revision")?,
                    to_i64(runtime.revision.value(), "checkpoint revision")?,
                    runtime.config.model,
                    serde_json::to_string(&runtime.config.mode).map_err(json_error)?,
                    runtime.title.as_ref().map(|title| title.value.as_str()),
                    runtime
                        .turn
                        .as_ref()
                        .map(|turn| to_i64(turn.id.value(), "turn id"))
                        .transpose()?,
                    runtime
                        .turn
                        .as_ref()
                        .map(|turn| turn_phase_name(turn.phase)),
                    i64::from(recovery_needed),
                    runtime_json,
                    summary_json,
                    runtime
                        .forked_from_session_id
                        .map(|id| to_i64(id.value(), "fork session id"))
                        .transpose()?,
                    (runtime.lifecycle == AgentSessionLifecycle::Archived)
                        .then(|| runtime.updated_at.to_string()),
                    runtime
                        .parent_session_id
                        .map(|id| to_i64(id.value(), "parent session id"))
                        .transpose()?,
                ],
            )
            .map_err(database_error)?;
    } else {
        project_delta_head(transaction, record)?;
    }
    transaction
        .execute(
            r#"
INSERT INTO agent_history_records(
    session_id, record_ordinal, start_byte_offset, end_byte_offset, session_revision
) VALUES (?, ?, ?, ?, ?)
            "#,
            params![
                to_i64(record.session_id.value(), "session id")?,
                to_i64(record.ordinal, "record ordinal")?,
                to_i64(start_offset, "record byte offset")?,
                to_i64(end_offset, "record byte offset")?,
                to_i64(record.session_revision.value(), "session revision")?,
            ],
        )
        .map_err(database_error)?;
    if let Some(base) = runtime.and_then(|runtime| runtime.history_base) {
        transaction
            .execute(
                r#"
INSERT INTO agent_session_lineage(
    child_session_id, parent_session_id, parent_end_ordinal_exclusive, parent_end_byte_offset
) VALUES (?, ?, ?, ?)
ON CONFLICT(child_session_id) DO UPDATE SET
    parent_session_id = excluded.parent_session_id,
    parent_end_ordinal_exclusive = excluded.parent_end_ordinal_exclusive,
    parent_end_byte_offset = excluded.parent_end_byte_offset
                "#,
                params![
                    to_i64(record.session_id.value(), "session id")?,
                    to_i64(base.session_id.value(), "parent session id")?,
                    to_i64(base.end_ordinal_exclusive, "lineage cutoff")?,
                    to_i64(base.end_byte_offset, "lineage byte offset")?,
                ],
            )
            .map_err(database_error)?;
    }
    for (mutation_index, mutation) in record.payload.history_mutations.iter().enumerate() {
        let item_order = record
            .ordinal
            .checked_mul(65_536)
            .and_then(|value| value.checked_add(mutation_index as u64))
            .ok_or_else(|| {
                AgentHistoryStoreError::CorruptHistory("item order exhausted".to_string())
            })?;
        match mutation {
            AgentHistoryMutation::AppendItem { item }
            | AgentHistoryMutation::UpsertItemSnapshot { item } => {
                project_item(transaction, record.session_id, record, item, item_order)?;
            }
            AgentHistoryMutation::SetModelContextBoundary { .. } => {}
        }
    }
    if let Some(turn) = runtime.and_then(|runtime| runtime.turn.as_ref()) {
        transaction
            .execute(
                r#"
UPDATE agent_turns SET
    last_record_ordinal = ?,
    status = ?,
    error_json = ?,
    completed_at = ?,
    final_answer_item_id = ?
WHERE session_id = ? AND turn_id = ?
                "#,
                params![
                    to_i64(record.ordinal, "record ordinal")?,
                    turn_phase_name(turn.phase),
                    turn.last_error
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(json_error)?,
                    turn.phase
                        .is_terminal()
                        .then(|| turn.updated_at.to_string()),
                    turn.final_answer_message_id
                        .map(|id| to_i64(id.value(), "message id"))
                        .transpose()?,
                    to_i64(record.session_id.value(), "session id")?,
                    to_i64(turn.id.value(), "turn id")?,
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

/// The compression chain's link a checkpoint carries, as a row: a snapshot
/// (or a record written before the chain, whose whole state is one) or the
/// state's end replaces the session's rows. Loading folds them.
fn project_compression_link(
    transaction: &rusqlite::Transaction<'_>,
    record: &AgentHistoryRecord,
    runtime: &hanafubuki_agent_kernel::PersistedAgentRuntimeState,
) -> Result<(), AgentHistoryStoreError> {
    let legacy;
    let link = match (&runtime.compression_update, &runtime.compression) {
        (Some(link), _) => link,
        (None, Some(state)) => {
            legacy = AgentCompressionUpdate::Snapshot(Box::new(state.clone()));
            &legacy
        }
        (None, None) => return Ok(()),
    };
    let session_id = to_i64(record.session_id.value(), "session id")?;
    if matches!(
        link,
        AgentCompressionUpdate::Snapshot(_) | AgentCompressionUpdate::Dropped
    ) {
        transaction
            .execute(
                "DELETE FROM agent_compression_links WHERE session_id = ?",
                params![session_id],
            )
            .map_err(database_error)?;
    }
    if *link != AgentCompressionUpdate::Dropped {
        transaction
            .execute(
                "INSERT OR REPLACE INTO agent_compression_links(session_id, record_ordinal, link_json) VALUES (?, ?, ?)",
                params![
                    session_id,
                    to_i64(record.ordinal, "record ordinal")?,
                    serde_json::to_string(link).map_err(json_error)?,
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

/// The runtime as the index keeps it: without its compression state, which
/// the chain's rows hold.
fn runtime_json(
    runtime: &hanafubuki_agent_kernel::PersistedAgentRuntimeState,
) -> Result<String, AgentHistoryStoreError> {
    if runtime.compression.is_none() && runtime.compression_update.is_none() {
        return serde_json::to_string(runtime).map_err(json_error);
    }
    serde_json::to_string(&hanafubuki_agent_kernel::PersistedAgentRuntimeState {
        compression: None,
        compression_update: None,
        ..runtime.clone()
    })
    .map_err(json_error)
}

fn project_delta_head(
    transaction: &rusqlite::Transaction<'_>,
    record: &AgentHistoryRecord,
) -> Result<(), AgentHistoryStoreError> {
    let encoded = transaction
        .query_row(
            "SELECT summary_json FROM agent_sessions WHERE session_id = ?",
            params![to_i64(record.session_id.value(), "session id")?],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| {
            AgentHistoryStoreError::CorruptHistory(
                "a history delta precedes its runtime checkpoint".to_string(),
            )
        })?;
    let mut summary: AgentSessionSummary = serde_json::from_str(&encoded).map_err(json_error)?;
    summary.revision = record.session_revision;
    summary.updated_at = record.session_updated_at;
    transaction
        .execute(
            r#"
UPDATE agent_sessions SET
    updated_at = ?,
    revision = ?,
    summary_json = ?
WHERE session_id = ?
            "#,
            params![
                record.session_updated_at.to_string(),
                to_i64(record.session_revision.value(), "revision")?,
                serde_json::to_string(&summary).map_err(json_error)?,
                to_i64(record.session_id.value(), "session id")?,
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn project_item(
    transaction: &rusqlite::Transaction<'_>,
    session_id: AgentSessionId,
    record: &AgentHistoryRecord,
    item: &AgentHistoryItem,
    item_order: u64,
) -> Result<(), AgentHistoryStoreError> {
    let item_json = serde_json::to_string(item).map_err(json_error)?;
    let item_type = conversation_item_type(&item.content);
    transaction
        .execute(
            r#"
INSERT INTO agent_history_items(
    session_id, turn_id, item_id, first_record_ordinal, first_item_order,
    item_revision, created_at, item_type, item_json, session_revision
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(session_id, item_id) DO UPDATE SET
    turn_id = excluded.turn_id,
    item_revision = excluded.item_revision,
    item_type = excluded.item_type,
    item_json = excluded.item_json
WHERE excluded.item_revision > agent_history_items.item_revision
            "#,
            params![
                to_i64(session_id.value(), "session id")?,
                item.turn_id
                    .map(|turn_id| to_i64(turn_id.value(), "turn id"))
                    .transpose()?,
                to_i64(item.item_id.value(), "item id")?,
                to_i64(record.ordinal, "record ordinal")?,
                to_i64(item_order, "item order")?,
                to_i64(item.item_revision, "item revision")?,
                record.recorded_at.to_string(),
                item_type,
                item_json,
                to_i64(record.session_revision.value(), "session revision")?,
            ],
        )
        .map_err(database_error)?;
    if let ConversationItem::TurnStart { turn_id } = item.content {
        transaction
            .execute(
                r#"
INSERT INTO agent_turns(
    session_id, turn_id, first_record_ordinal, first_item_order, status, started_at
) VALUES (?, ?, ?, ?, 'sampling', ?)
ON CONFLICT(session_id, turn_id) DO NOTHING
                "#,
                params![
                    to_i64(session_id.value(), "session id")?,
                    to_i64(turn_id.value(), "turn id")?,
                    to_i64(record.ordinal, "record ordinal")?,
                    to_i64(item_order, "item order")?,
                    record.recorded_at.to_string(),
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

fn session_summary(
    runtime: &hanafubuki_agent_kernel::PersistedAgentRuntimeState,
) -> AgentSessionSummary {
    AgentSessionSummary {
        session_id: runtime.id,
        forked_from_session_id: runtime.forked_from_session_id,
        parent_session_id: runtime.parent_session_id,
        origin: runtime.origin,
        cwd: runtime.cwd.clone(),
        branch: runtime.branch.clone(),
        generation: runtime.generation,
        revision: runtime.revision,
        provider_id: runtime.config.provider_id.clone(),
        provider_protocol: runtime.config.provider_protocol,
        model: runtime.config.model.clone(),
        mode: runtime.config.mode,
        reasoning_effort: runtime.config.reasoning_effort.clone(),
        title: runtime.title.clone(),
        lifecycle: runtime.lifecycle,
        usage: runtime.usage,
        compaction: runtime.compaction.clone(),
        latest_turn_phase: runtime.turn.as_ref().map(|turn| turn.phase),
        created_at: runtime.created_at,
        updated_at: runtime.updated_at,
        recovery_marker: runtime.turn.as_ref().and_then(|turn| turn.recovery),
    }
}

pub(in crate::store) fn runtime_recovery_needed(
    runtime: &hanafubuki_agent_kernel::PersistedAgentRuntimeState,
) -> bool {
    runtime
        .turn
        .as_ref()
        .is_some_and(|turn| !turn.phase.is_terminal() || turn.reasoning.is_active())
        || runtime
            .compaction
            .as_ref()
            .is_some_and(|compaction| !compaction.status.is_terminal())
        || runtime.pending_user_input.is_some()
        || runtime.activities.iter().any(|activity| {
            matches!(
                activity.phase,
                hanafubuki_agent_kernel::AgentActivityPhase::Running
                    | hanafubuki_agent_kernel::AgentActivityPhase::WaitingForApproval
                    | hanafubuki_agent_kernel::AgentActivityPhase::WaitingForCleanup
            )
        })
}
