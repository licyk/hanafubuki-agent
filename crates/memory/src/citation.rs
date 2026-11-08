use serde::{Deserialize, Serialize};

use crate::{AgentMemoryError, MAX_MEMORY_CITATIONS};

pub const MEMORY_CITATION_OPEN: &str = "<hanafubuki-memory-citations>";
pub const MEMORY_CITATION_CLOSE: &str = "</hanafubuki-memory-citations>";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryCitation {
    pub resource_id: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub note: String,
    pub source_session_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMemoryCitations {
    pub visible_text: String,
    pub citations: Vec<AgentMemoryCitation>,
}

/// Parses only one complete citation block at the very end. Malformed blocks
/// remain visible so a parser failure can never silently remove an answer.
pub fn parse_memory_citations(text: &str) -> Result<ParsedMemoryCitations, AgentMemoryError> {
    let trimmed = text.trim_end();
    if !trimmed.ends_with(MEMORY_CITATION_CLOSE) {
        return Ok(ParsedMemoryCitations {
            visible_text: text.to_string(),
            citations: Vec::new(),
        });
    }
    let Some(start) = trimmed.rfind(MEMORY_CITATION_OPEN) else {
        return Ok(ParsedMemoryCitations {
            visible_text: text.to_string(),
            citations: Vec::new(),
        });
    };
    let payload_start = start + MEMORY_CITATION_OPEN.len();
    let payload_end = trimmed.len() - MEMORY_CITATION_CLOSE.len();
    let citations: Vec<AgentMemoryCitation> =
        serde_json::from_str(trimmed[payload_start..payload_end].trim()).map_err(|_| {
            AgentMemoryError::InvalidInput("malformed memory citation block".into())
        })?;
    if citations.len() > MAX_MEMORY_CITATIONS
        || citations.iter().any(|citation| {
            citation.resource_id.is_empty()
                || citation.resource_id.len() > 256
                || citation.note.len() > 512
                || citation.source_session_ids.len() > 64
                || citation
                    .start_line
                    .zip(citation.end_line)
                    .is_some_and(|(start, end)| start == 0 || end < start)
        })
    {
        return Err(AgentMemoryError::InvalidInput(
            "memory citation exceeds bounds".into(),
        ));
    }
    Ok(ParsedMemoryCitations {
        visible_text: trimmed[..start].trim_end().to_string(),
        citations,
    })
}
