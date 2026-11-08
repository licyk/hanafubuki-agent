use super::cursor::{
    decode_cursor, encode_cursor, opposite_direction, validate_page_size, TranscriptCursor,
};
use super::lineage::visible_item_anchor_exists;
use crate::store::*;

impl FileAgentHistoryStore {
    pub(in crate::store) fn read_visible_items(
        &self,
        session_id: AgentSessionId,
        through_revision: Option<AgentRevision>,
    ) -> Result<Vec<(AgentSessionId, u64, AgentHistoryItem)>, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let lineage = self.resolve_lineage(session_id, &connection)?;
        let mut result = Vec::new();
        for (logical_segment, segment) in lineage.into_iter().enumerate() {
            let mut sql = String::from(
                "SELECT first_item_order, item_json FROM agent_history_items WHERE session_id = ?",
            );
            if segment.end_record_ordinal_exclusive.is_some() {
                sql.push_str(" AND first_record_ordinal < ?");
            }
            let segment_revision = (segment.session_id == session_id)
                .then_some(through_revision)
                .flatten();
            if segment_revision.is_some() {
                sql.push_str(" AND session_revision <= ?");
            }
            sql.push_str(" ORDER BY first_item_order ASC");
            let mut statement = connection.prepare(&sql).map_err(database_error)?;
            let session_value = to_i64(segment.session_id.value(), "session id")?;
            let cutoff = segment
                .end_record_ordinal_exclusive
                .map(|value| to_i64(value, "lineage cutoff"))
                .transpose()?;
            let revision = segment_revision
                .map(|value| to_i64(value.value(), "session revision"))
                .transpose()?;
            let mut rows = match (cutoff, revision) {
                (Some(cutoff), Some(revision)) => statement
                    .query(params![session_value, cutoff, revision])
                    .map_err(database_error)?,
                (Some(cutoff), None) => statement
                    .query(params![session_value, cutoff])
                    .map_err(database_error)?,
                (None, Some(revision)) => statement
                    .query(params![session_value, revision])
                    .map_err(database_error)?,
                (None, None) => statement
                    .query(params![session_value])
                    .map_err(database_error)?,
            };
            while let Some(row) = rows.next().map_err(database_error)? {
                let item_order = from_i64(row.get(0).map_err(database_error)?, "item order")?;
                let encoded: String = row.get(1).map_err(database_error)?;
                let mut item: AgentHistoryItem =
                    serde_json::from_str(&encoded).map_err(json_error)?;
                item.source_session_id = segment.session_id;
                item.logical_segment = u32::try_from(logical_segment).map_err(|_| {
                    AgentHistoryStoreError::CorruptHistory(
                        "Agent history lineage depth exceeds u32".to_string(),
                    )
                })?;
                item.logical_order = item_order;
                result.push((segment.session_id, item_order, item));
            }
        }
        Ok(result)
    }

    pub(in crate::store) fn read_visible_items_page(
        &self,
        session_id: AgentSessionId,
        through_revision: AgentRevision,
        cursor: Option<&TranscriptCursor>,
        direction: AgentHistorySortDirection,
        turn_id: Option<AgentTurnId>,
        limit: usize,
    ) -> Result<Vec<(AgentSessionId, u64, AgentHistoryItem)>, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let lineage = self.resolve_lineage(session_id, &connection)?;
        let cursor_index = cursor
            .map(|cursor| {
                lineage
                    .iter()
                    .position(|segment| segment.session_id == cursor.physical_session_id)
                    .ok_or_else(|| {
                        AgentHistoryStoreError::InvalidCursor(
                            "transcript cursor references a hidden lineage segment".to_string(),
                        )
                    })
            })
            .transpose()?;
        if let (Some(cursor), Some(index)) = (cursor, cursor_index) {
            if !visible_item_anchor_exists(
                &connection,
                &lineage[index],
                session_id,
                through_revision,
                turn_id,
                cursor.item_order,
            )? {
                return Err(AgentHistoryStoreError::InvalidCursor(
                    "transcript anchor is not visible".to_string(),
                ));
            }
        }

        let mut indexes = match direction {
            AgentHistorySortDirection::Ascending => {
                (cursor_index.unwrap_or(0)..lineage.len()).collect::<Vec<_>>()
            }
            AgentHistorySortDirection::Descending => {
                let end = cursor_index.unwrap_or_else(|| lineage.len().saturating_sub(1));
                (0..=end).rev().collect::<Vec<_>>()
            }
        };
        if lineage.is_empty() {
            indexes.clear();
        }
        let mut result = Vec::with_capacity(limit);
        for index in indexes {
            let segment = &lineage[index];
            let remaining = limit.saturating_sub(result.len());
            if remaining == 0 {
                break;
            }
            let mut sql = String::from(
                "SELECT first_item_order, item_json FROM agent_history_items WHERE session_id = ?",
            );
            let mut values = vec![Value::Integer(to_i64(
                segment.session_id.value(),
                "session id",
            )?)];
            if let Some(cutoff) = segment.end_record_ordinal_exclusive {
                sql.push_str(" AND first_record_ordinal < ?");
                values.push(Value::Integer(to_i64(cutoff, "lineage cutoff")?));
            }
            if segment.session_id == session_id {
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
            if let Some(cursor) =
                cursor.filter(|cursor| cursor.physical_session_id == segment.session_id)
            {
                let comparison = match (direction, cursor.include_anchor) {
                    (AgentHistorySortDirection::Ascending, true) => " >= ?",
                    (AgentHistorySortDirection::Ascending, false) => " > ?",
                    (AgentHistorySortDirection::Descending, true) => " <= ?",
                    (AgentHistorySortDirection::Descending, false) => " < ?",
                };
                sql.push_str(" AND first_item_order");
                sql.push_str(comparison);
                values.push(Value::Integer(to_i64(cursor.item_order, "item order")?));
            }
            sql.push_str(match direction {
                AgentHistorySortDirection::Ascending => " ORDER BY first_item_order ASC LIMIT ?",
                AgentHistorySortDirection::Descending => " ORDER BY first_item_order DESC LIMIT ?",
            });
            values.push(Value::Integer(to_i64(remaining as u64, "page size")?));
            let mut statement = connection.prepare(&sql).map_err(database_error)?;
            let rows = statement
                .query_map(params_from_iter(values.iter()), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(database_error)?;
            for row in rows {
                let (raw_order, encoded) = row.map_err(database_error)?;
                let item_order = from_i64(raw_order, "item order")?;
                let mut item: AgentHistoryItem =
                    serde_json::from_str(&encoded).map_err(json_error)?;
                item.source_session_id = segment.session_id;
                item.logical_segment = u32::try_from(index).map_err(|_| {
                    AgentHistoryStoreError::CorruptHistory(
                        "Agent history lineage depth exceeds u32".to_string(),
                    )
                })?;
                item.logical_order = item_order;
                result.push((segment.session_id, item_order, item));
            }
        }
        Ok(result)
    }

    pub(in crate::store) fn list_scoped_items_page(
        &self,
        request: AgentTranscriptPageRequest,
        turn_id: Option<AgentTurnId>,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        if let Some(error) = self.inner.projection_lag.lock().get(&request.session_id) {
            return Err(AgentHistoryStoreError::ProjectionUnavailable(
                error.reason().to_string(),
            ));
        }
        validate_page_size(request.page_size, AGENT_TRANSCRIPT_PAGE_SIZE_MAX)?;
        let head_revision = self.load_head_revision_inner(request.session_id)?;
        let through_revision = request.through_revision.unwrap_or(head_revision);
        if through_revision.value() > head_revision.value() {
            return Err(AgentHistoryStoreError::InvalidCursor(
                "transcript revision is newer than the session".to_string(),
            ));
        }
        let cursor = request
            .cursor
            .as_deref()
            .map(decode_cursor::<TranscriptCursor>)
            .transpose()?;
        if let Some(cursor) = &cursor {
            if cursor.version != 1
                || cursor.requested_session_id != request.session_id
                || cursor.direction != request.direction
                || cursor.through_revision != through_revision
                || cursor.turn_id != turn_id
            {
                return Err(AgentHistoryStoreError::InvalidCursor(
                    "transcript cursor does not match the request scope".to_string(),
                ));
            }
        }
        let mut visible = self.read_visible_items_page(
            request.session_id,
            through_revision,
            cursor.as_ref(),
            request.direction,
            turn_id,
            request.page_size.saturating_add(1),
        )?;
        let has_more = visible.len() > request.page_size;
        visible.truncate(request.page_size);
        let cursor_for = |physical: AgentSessionId,
                          order: u64,
                          direction: AgentHistorySortDirection,
                          include_anchor: bool| {
            encode_cursor(&TranscriptCursor {
                version: 1,
                requested_session_id: request.session_id,
                physical_session_id: physical,
                item_order: order,
                direction,
                through_revision,
                include_anchor,
                turn_id,
            })
        };
        let backwards_cursor = visible
            .first()
            .map(|(physical, order, _)| {
                cursor_for(
                    *physical,
                    *order,
                    opposite_direction(request.direction),
                    true,
                )
            })
            .transpose()?;
        let next_cursor = if has_more {
            visible
                .last()
                .map(|(physical, order, _)| cursor_for(*physical, *order, request.direction, false))
                .transpose()?
        } else {
            None
        };
        let items = visible.into_iter().map(|(_, _, item)| item).collect();
        let (older_cursor, newer_cursor) = match request.direction {
            AgentHistorySortDirection::Descending => (next_cursor, backwards_cursor),
            AgentHistorySortDirection::Ascending => (backwards_cursor, next_cursor),
        };
        Ok(AgentTranscriptPage {
            items,
            older_cursor,
            newer_cursor,
            through_revision,
        })
    }

    pub(in crate::store) fn list_items_page_inner(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        self.list_scoped_items_page(request, None)
    }

    pub(in crate::store) fn list_turn_items_page_inner(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        self.list_scoped_items_page(
            AgentTranscriptPageRequest {
                session_id: request.session_id,
                page_size: request.page_size,
                cursor: request.cursor,
                direction: request.direction,
                through_revision: request.through_revision,
            },
            Some(request.turn_id),
        )
    }
}
