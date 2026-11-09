use crate::store::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::store) struct SessionCursor {
    pub(in crate::store) version: u32,
    pub(in crate::store) updated_at: u128,
    pub(in crate::store) session_id: AgentSessionId,
    pub(in crate::store) lifecycle: AgentSessionLifecycle,
    pub(in crate::store) direction: AgentHistorySortDirection,
    pub(in crate::store) search: Option<String>,
    pub(in crate::store) parent: hanafubuki_agent_kernel::AgentSessionParentFilter,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::store) struct TranscriptCursor {
    pub(in crate::store) version: u32,
    pub(in crate::store) requested_session_id: AgentSessionId,
    pub(in crate::store) physical_session_id: AgentSessionId,
    pub(in crate::store) item_order: u64,
    pub(in crate::store) direction: AgentHistorySortDirection,
    pub(in crate::store) through_revision: AgentRevision,
    pub(in crate::store) include_anchor: bool,
    pub(in crate::store) turn_id: Option<AgentTurnId>,
}

pub(in crate::store) fn validate_page_size(
    page_size: usize,
    maximum: usize,
) -> Result<(), AgentHistoryStoreError> {
    if page_size == 0 || page_size > maximum {
        return Err(AgentHistoryStoreError::InvalidCursor(format!(
            "page size must be between 1 and {maximum}"
        )));
    }
    Ok(())
}

pub(in crate::store) fn encode_cursor<T: Serialize>(
    value: &T,
) -> Result<String, AgentHistoryStoreError> {
    let bytes = serde_json::to_vec(value).map_err(json_error)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(in crate::store) fn decode_cursor<T: for<'de> Deserialize<'de>>(
    value: &str,
) -> Result<T, AgentHistoryStoreError> {
    if !value.len().is_multiple_of(2) {
        return Err(AgentHistoryStoreError::InvalidCursor(
            "cursor hex length is invalid".to_string(),
        ));
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|error| {
                AgentHistoryStoreError::InvalidCursor(format!("cursor is not hex: {error}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AgentHistoryStoreError::InvalidCursor(error.to_string()))
}

pub(in crate::store) fn opposite_direction(
    direction: AgentHistorySortDirection,
) -> AgentHistorySortDirection {
    match direction {
        AgentHistorySortDirection::Ascending => AgentHistorySortDirection::Descending,
        AgentHistorySortDirection::Descending => AgentHistorySortDirection::Ascending,
    }
}
