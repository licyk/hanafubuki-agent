use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde_json::{json, Value};

use crate::{ModelError, ModelResult};

pub(super) fn function_response(
    output: &ToolOutput,
    name: &str,
    id: Option<&str>,
    order: usize,
    separate_media: bool,
) -> ModelResult<(Value, Vec<Value>)> {
    output
        .validate()
        .map_err(|e| ModelError::InvalidRequest(e.to_string()))?;
    let mut content = Vec::new();
    let mut parts = Vec::new();
    for item in &output.content {
        let (mime, data) = match item {
            ToolOutputContent::Image {
                mime_type, data, ..
            } => (mime_type, data),
            ToolOutputContent::EmbeddedResource {
                mime_type: Some(mime),
                data: Some(data),
                text: None,
                ..
            } if mime.starts_with("image/") => (mime, data),
            ToolOutputContent::Audio { mime_type, .. } => {
                content.push(json!({"type":"unsupported_audio","mimeType":mime_type}));
                continue;
            }
            ToolOutputContent::EmbeddedResource {
                uri,
                mime_type,
                text,
                data,
            } => {
                content.push(json!({"type":"embedded_resource","uri":uri,"mimeType":mime_type,"text":text,"binaryDataOmitted":data.is_some()}));
                continue;
            }
            ToolOutputContent::ResourceLink { .. } => {
                content.push(json!(item));
                continue;
            }
            ToolOutputContent::Text { text } => {
                content.push(json!(text));
                continue;
            }
            ToolOutputContent::Structured { value } => {
                content.push(value.clone());
                continue;
            }
        };
        let display = format!("attachment-{}-{}", order + 1, parts.len() + 1);
        content.push(if separate_media {
            json!(format!("See attached {mime} image {display}."))
        } else {
            json!({"$ref": display})
        });
        parts.push(json!({"inlineData":{"displayName":display,"mimeType":mime,"data":data}}));
    }
    if let Some(structured) = &output.structured_content {
        if !structured.is_null() && !content.contains(structured) {
            content.push(structured.clone());
        }
    }
    let output_value = if content.len() == 1 {
        content.remove(0)
    } else {
        Value::Array(content)
    };
    let key = if output.is_error { "error" } else { "output" };
    let mut result = json!({"name":name,"response":{(key):output_value}});
    if !separate_media && !parts.is_empty() {
        result["parts"] = Value::Array(std::mem::take(&mut parts));
    }
    if let Some(id) = id {
        result["id"] = json!(id);
    }
    Ok((json!({"functionResponse":result}), parts))
}
