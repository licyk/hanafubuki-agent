use crate::store::*;

#[derive(Clone, Debug)]
pub(in crate::store) struct HistorySegment {
    pub(in crate::store) session_id: AgentSessionId,
    pub(in crate::store) end_record_ordinal_exclusive: Option<u64>,
}

impl FileAgentHistoryStore {
    pub(in crate::store) fn resolve_lineage(
        &self,
        requested_session_id: AgentSessionId,
        connection: &Connection,
    ) -> Result<Vec<HistorySegment>, AgentHistoryStoreError> {
        let mut segments = Vec::new();
        let mut seen = HashSet::new();
        let mut session_id = requested_session_id;
        let mut end = None;
        loop {
            if !seen.insert(session_id) {
                return Err(AgentHistoryStoreError::CorruptHistory(format!(
                    "lineage cycle detected for session {requested_session_id}"
                )));
            }
            let exists = connection
                .query_row(
                    "SELECT 1 FROM agent_sessions WHERE session_id = ?",
                    params![to_i64(session_id.value(), "session id")?],
                    |_| Ok(()),
                )
                .optional()
                .map_err(database_error)?
                .is_some();
            if !exists {
                return Err(AgentHistoryStoreError::CorruptHistory(format!(
                    "lineage source session {session_id} is missing"
                )));
            }
            segments.push(HistorySegment {
                session_id,
                end_record_ordinal_exclusive: end,
            });
            let parent = connection
                .query_row(
                    "SELECT parent_session_id, parent_end_ordinal_exclusive FROM agent_session_lineage WHERE child_session_id = ?",
                    params![to_i64(session_id.value(), "session id")?],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()
                .map_err(database_error)?;
            let Some((parent_id, cutoff)) = parent else {
                break;
            };
            session_id = AgentSessionId::new(from_i64(parent_id, "parent session id")?);
            end = Some(from_i64(cutoff, "lineage cutoff")?);
        }
        segments.reverse();
        Ok(segments)
    }

    pub(in crate::store) fn lookup_before_turn_base(
        &self,
        source_session_id: AgentSessionId,
        before_turn_id: AgentTurnId,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let lineage = self.resolve_lineage(source_session_id, &connection)?;
        for segment in lineage.into_iter().rev() {
            let row = connection
                .query_row(
                    "SELECT first_record_ordinal FROM agent_turns WHERE session_id = ? AND turn_id = ?",
                    params![
                        to_i64(segment.session_id.value(), "session id")?,
                        to_i64(before_turn_id.value(), "turn id")?,
                    ],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(database_error)?;
            let Some(record_ordinal) = row else {
                continue;
            };
            let record_ordinal = from_i64(record_ordinal, "turn record ordinal")?;
            if segment
                .end_record_ordinal_exclusive
                .is_some_and(|cutoff| record_ordinal >= cutoff)
            {
                continue;
            }
            let byte_offset = connection
                .query_row(
                    "SELECT start_byte_offset FROM agent_history_records WHERE session_id = ? AND record_ordinal = ?",
                    params![
                        to_i64(segment.session_id.value(), "session id")?,
                        to_i64(record_ordinal, "record ordinal")?,
                    ],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(database_error)?;
            return Ok(AgentHistoryBase {
                session_id: segment.session_id,
                end_ordinal_exclusive: record_ordinal,
                end_byte_offset: from_i64(byte_offset, "record byte offset")?,
            });
        }
        Err(AgentHistoryStoreError::NotFound {
            session_id: source_session_id,
        })
    }

    pub(in crate::store) fn lookup_head_base(
        &self,
        source_session_id: AgentSessionId,
        source_revision: AgentRevision,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let source_value = to_i64(source_session_id.value(), "session id")?;
        let projected = connection
            .query_row(
                "SELECT next_ordinal, next_jsonl_byte_offset, last_projected_session_revision FROM agent_history_projection_state WHERE session_id = ?",
                params![source_value],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()
            .map_err(database_error)?
            .ok_or(AgentHistoryStoreError::NotFound {
                session_id: source_session_id,
            })?;
        if from_i64(projected.2, "projected session revision")? != source_revision.value() {
            return Err(AgentHistoryStoreError::StaleRevision);
        }
        Ok(AgentHistoryBase {
            session_id: source_session_id,
            end_ordinal_exclusive: from_i64(projected.0, "history head ordinal")?,
            end_byte_offset: from_i64(projected.1, "history head byte offset")?,
        })
    }
}

pub(in crate::store) fn visible_item_anchor_exists(
    connection: &Connection,
    segment: &HistorySegment,
    requested_session_id: AgentSessionId,
    through_revision: AgentRevision,
    turn_id: Option<AgentTurnId>,
    item_order: u64,
) -> Result<bool, AgentHistoryStoreError> {
    let mut sql = String::from(
        "SELECT 1 FROM agent_history_items WHERE session_id = ? AND first_item_order = ?",
    );
    let mut values = vec![
        Value::Integer(to_i64(segment.session_id.value(), "session id")?),
        Value::Integer(to_i64(item_order, "item order")?),
    ];
    if let Some(cutoff) = segment.end_record_ordinal_exclusive {
        sql.push_str(" AND first_record_ordinal < ?");
        values.push(Value::Integer(to_i64(cutoff, "lineage cutoff")?));
    }
    if segment.session_id == requested_session_id {
        sql.push_str(" AND session_revision <= ?");
        values.push(Value::Integer(to_i64(
            through_revision.value(),
            "session revision",
        )?));
    }
    if let Some(turn_id) = turn_id {
        sql.push_str(" AND turn_id = ?");
        values.push(Value::Integer(to_i64(turn_id.value(), "turn id")?));
    }
    connection
        .query_row(&sql, params_from_iter(values.iter()), |_| Ok(()))
        .optional()
        .map(|value| value.is_some())
        .map_err(database_error)
}
