//! Readable memory resources, matches and notes, live and from history.

use hanafubuki_agent_contracts::{
    structured_output, AgentListingEntry, AgentListingEntryKind, AgentSearchMatch,
    AgentToolDisplay, AgentToolPresenter,
};
use hanafubuki_agent_memory::{AgentMemoryReadResult, AgentMemoryResource, AgentMemorySearchMatch};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde_json::Value;

pub(super) struct MemoryPresenter;

impl AgentToolPresenter for MemoryPresenter {
    fn tools(&self) -> &[&'static str] {
        &[
            "list_agent_memories",
            "search_agent_memories",
            "read_agent_memory",
            "append_agent_memory_note",
        ]
    }

    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        let key = match tool {
            "search_agent_memories" => "query",
            "read_agent_memory" => "resourceId",
            "append_agent_memory_note" => "operation",
            _ => return None,
        };
        arguments.get(key)?.as_str().map(str::to_string)
    }

    fn display(
        &self,
        tool: &str,
        arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        if output.is_error {
            return None;
        }
        let value = structured_output(output)?;
        Some(match tool {
            "list_agent_memories" => AgentToolDisplay::Listing {
                path: "memories".into(),
                entries: value
                    .get("resources")?
                    .as_array()?
                    .iter()
                    .filter_map(|value| {
                        serde_json::from_value::<AgentMemoryResource>(value.clone()).ok()
                    })
                    .map(|resource| AgentListingEntry {
                        path: resource.id,
                        kind: AgentListingEntryKind::File,
                        size: Some(resource.bytes as u64),
                    })
                    .collect(),
                truncated: false,
            },
            "search_agent_memories" => AgentToolDisplay::Search {
                query: arguments.get("query")?.as_str()?.to_string(),
                path: None,
                matches: value
                    .get("matches")?
                    .as_array()?
                    .iter()
                    .filter_map(|value| {
                        serde_json::from_value::<AgentMemorySearchMatch>(value.clone()).ok()
                    })
                    .map(|found| AgentSearchMatch {
                        path: found.resource_id,
                        line: found.line as u64,
                        text: found.excerpt,
                    })
                    .collect(),
                truncated: false,
            },
            "read_agent_memory" => {
                let read: AgentMemoryReadResult = serde_json::from_value(value.clone()).ok()?;
                AgentToolDisplay::FileRead {
                    path: read.resource_id,
                    start_line: Some(read.start_line as u64),
                    text: read.content,
                    truncated: read.truncated,
                }
            }
            "append_agent_memory_note" => AgentToolDisplay::Text {
                text: format!(
                    "Memory note queued for consolidation.\nOperation: {}\nNote: {}",
                    value.get("operation")?.as_str()?,
                    value.get("noteId")?.as_str()?
                ),
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests;
