use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent, ToolOutputImageDetail};
use serde_json::{json, Value};

use crate::{ModelError, ModelResult};

pub(super) fn content(output: &ToolOutput) -> ModelResult<Vec<Value>> {
    output
        .validate()
        .map_err(|error| ModelError::InvalidRequest(error.to_string()))?;
    let mut parts = Vec::with_capacity(output.content.len());
    for item in &output.content {
        let part = match item {
            ToolOutputContent::Text { text } => json!({"type":"text","text":text}),
            ToolOutputContent::Structured { value } => json_text(value),
            ToolOutputContent::Image {
                mime_type,
                data,
                detail,
            } => {
                let mut image = media("image", mime_type, data);
                match detail {
                    Some(ToolOutputImageDetail::Low) => image["resolution"] = json!("low"),
                    Some(ToolOutputImageDetail::High | ToolOutputImageDetail::Original) => {
                        image["resolution"] = json!("high")
                    }
                    _ => {}
                }
                image
            }
            ToolOutputContent::Audio { mime_type, data } => media("audio", mime_type, data),
            ToolOutputContent::ResourceLink { .. } => json_text(&json!(item)),
            ToolOutputContent::EmbeddedResource {
                uri,
                mime_type,
                text,
                data,
            } => match (text, mime_type.as_deref(), data) {
                (None, Some(mime), Some(data)) if mime.starts_with("image/") => {
                    media("image", mime, data)
                }
                (None, Some(mime), Some(data)) if mime.starts_with("audio/") => {
                    media("audio", mime, data)
                }
                (None, Some("application/pdf"), Some(data)) => {
                    media("document", "application/pdf", data)
                }
                _ => json_text(
                    &json!({"type":"embedded_resource","uri":uri,"mimeType":mime_type,"text":text,"binaryDataOmitted":data.is_some()}),
                ),
            },
        };
        parts.push(part);
    }
    if let Some(value) = output
        .structured_content
        .as_ref()
        .filter(|value| !value.is_null())
    {
        if !output
            .content
            .iter()
            .any(|item| matches!(item,ToolOutputContent::Structured{value:other} if value==other))
        {
            parts.push(json_text(value));
        }
    }
    Ok(parts)
}

fn media(kind: &str, mime: &str, data: &str) -> Value {
    json!({"type":kind,"mime_type":mime,"data":data})
}

fn json_text(value: &Value) -> Value {
    json!({"type":"text","text":value.to_string()})
}
