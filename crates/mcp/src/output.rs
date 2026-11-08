use hanafubuki_agent_tool_contracts::{
    canonical_base64, canonical_media_type, ToolOutput, ToolOutputContent, MAX_TOOL_OUTPUT_ITEMS,
};
use rmcp::model::{
    CallToolResult, ContentBlock, ReadResourceResult, ResourceContents, ServerResult,
};
use serde::Deserialize;
use serde_json::Value;

use crate::McpError;

/// Characters of an unsupported block's type name kept in its placeholder.
const MAX_UNSUPPORTED_TYPE_CHARS: usize = 64;

/// Converts a tools/call result item by item: a block this client does not
/// support becomes a placeholder text item (blocks rmcp cannot decode were
/// already replaced by [`decode_call_tool_result_leniently`]), items past
/// [`MAX_TOOL_OUTPUT_ITEMS`] are replaced by one note, media types lose their
/// parameters and base64 regains its padding. The byte bound and the
/// image/audio type checks of [`ToolOutput::validate`] still fail the result.
pub fn convert_tool_output(result: CallToolResult) -> Result<ToolOutput, McpError> {
    let content = bounded_items(result.content.into_iter().map(|item| match item {
        ContentBlock::Text(text) => ToolOutputContent::Text { text: text.text },
        ContentBlock::Image(image) => ToolOutputContent::Image {
            mime_type: canonical_media_type(&image.mime_type),
            data: canonical_base64(image.data),
            detail: None,
        },
        ContentBlock::Audio(audio) => ToolOutputContent::Audio {
            mime_type: canonical_media_type(&audio.mime_type),
            data: canonical_base64(audio.data),
        },
        ContentBlock::Resource(resource) => convert_resource(resource.resource),
        ContentBlock::ResourceLink(resource) => ToolOutputContent::ResourceLink {
            name: resource.name,
            uri: resource.uri,
            mime_type: resource.mime_type,
        },
        other => unsupported(&other),
    }));
    let output = ToolOutput {
        content,
        structured_content: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
    };
    output.validate().map_err(|_| McpError::LimitExceeded)?;
    Ok(output)
}

/// Converts a resources/read result entry by entry, like
/// [`convert_tool_output`]: an entry rmcp cannot decode becomes the
/// placeholder text `[unsupported MCP content: resource]` (or its `type`,
/// when it has one) and the rest are kept. A result that is not a
/// resources/read result still fails as a whole.
pub(crate) fn convert_read_resource_output(result: ServerResult) -> Result<ToolOutput, McpError> {
    let content = match result {
        ServerResult::ReadResourceResult(result) => {
            result.contents.into_iter().map(convert_resource).collect()
        }
        // rmcp falls back to a custom result when one entry does not decode.
        ServerResult::CustomResult(raw) => {
            decode_read_resource_leniently(raw.0).ok_or_else(|| {
                McpError::Protocol("MCP server returned an unexpected response type".into())
            })?
        }
        _ => {
            return Err(McpError::Protocol(
                "MCP server returned an unexpected response type".into(),
            ))
        }
    };
    let output = ToolOutput {
        content: bounded_items(content.into_iter()),
        structured_content: None,
        is_error: false,
    };
    output.validate().map_err(|_| McpError::LimitExceeded)?;
    Ok(output)
}

/// Converts each entry of a resources/read result that rmcp could not
/// decode, after checking that the result without its entries is one by
/// rmcp's rules. `None` when it is not.
fn decode_read_resource_leniently(mut raw: Value) -> Option<Vec<ToolOutputContent>> {
    let entries = match raw.get_mut("contents") {
        Some(Value::Array(entries)) => std::mem::take(entries),
        _ => return None,
    };
    ReadResourceResult::deserialize(&raw).ok()?;
    Some(
        entries
            .iter()
            .map(|entry| match ResourceContents::deserialize(entry) {
                Ok(resource) => convert_resource(resource),
                Err(_) => ToolOutputContent::Text {
                    text: placeholder(entry, "resource"),
                },
            })
            .collect(),
    )
}

/// Keeps every item when they fit, otherwise the first
/// `MAX_TOOL_OUTPUT_ITEMS - 1` and a note saying how many were omitted.
fn bounded_items(
    items: impl ExactSizeIterator<Item = ToolOutputContent>,
) -> Vec<ToolOutputContent> {
    let total = items.len();
    if total <= MAX_TOOL_OUTPUT_ITEMS {
        return items.collect();
    }
    let kept = MAX_TOOL_OUTPUT_ITEMS - 1;
    let mut content = items.take(kept).collect::<Vec<_>>();
    content.push(ToolOutputContent::Text {
        text: format!(
            "[{} more MCP content items omitted; at most {MAX_TOOL_OUTPUT_ITEMS} are kept]",
            total - kept
        ),
    });
    content
}

fn convert_resource(resource: ResourceContents) -> ToolOutputContent {
    match resource {
        ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            ..
        } => ToolOutputContent::EmbeddedResource {
            uri,
            mime_type,
            text: Some(text),
            data: None,
        },
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } => ToolOutputContent::EmbeddedResource {
            uri,
            mime_type: mime_type.as_deref().map(canonical_media_type),
            text: None,
            data: Some(canonical_base64(blob)),
        },
        other => unsupported(&other),
    }
}

/// Decodes a tools/call result that rmcp could not, because of individual
/// content blocks: each block that does not decode becomes a placeholder,
/// then the result is decoded by rmcp's own rules. `None` when it is still
/// not a tools/call result.
pub(crate) fn decode_call_tool_result_leniently(mut raw: Value) -> Option<CallToolResult> {
    if let Some(items) = raw.get_mut("content").and_then(Value::as_array_mut) {
        for item in items {
            if ContentBlock::deserialize(&*item).is_err() {
                *item =
                    serde_json::to_value(ContentBlock::text(placeholder(item, "unknown"))).ok()?;
            }
        }
    }
    CallToolResult::deserialize(&raw).ok()
}

fn unsupported(item: &impl serde::Serialize) -> ToolOutputContent {
    let text = serde_json::to_value(item)
        .map(|value| placeholder(&value, "unknown"))
        .unwrap_or_else(|_| placeholder(&Value::Null, "unknown"));
    ToolOutputContent::Text { text }
}

/// A placeholder naming the item's type (`default_kind` when it has none),
/// bounded and restricted to identifier characters; nothing else of the
/// item is kept.
fn placeholder(item: &Value, default_kind: &str) -> String {
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '-' | '.' | '/'))
        .take(MAX_UNSUPPORTED_TYPE_CHARS)
        .collect::<String>();
    format!(
        "[unsupported MCP content: {}]",
        if kind.is_empty() { default_kind } else { &kind }
    )
}
