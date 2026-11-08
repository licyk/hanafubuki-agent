use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent, ToolOutputImageDetail};
use serde_json::{json, Value};

use crate::{ModelError, ModelResult};

pub(crate) const CHAT_TOOL_IMAGE_ATTACHED_MARKER: &str =
    "[hanafubuki-agent: tool result image attached in the following user message]";

pub(crate) struct ChatToolOutput {
    pub(crate) tool_content: String,
    pub(crate) image_parts: Vec<Value>,
}

pub(crate) fn legacy_text(output: &ToolOutput) -> ModelResult<String> {
    output
        .legacy_text_projection()
        .map_err(|error| ModelError::InvalidRequest(error.to_string()))
}

pub(crate) fn chat_output(output: &ToolOutput) -> ModelResult<ChatToolOutput> {
    output
        .validate()
        .map_err(|error| ModelError::InvalidRequest(error.to_string()))?;
    let mut image_parts = Vec::new();
    let mut projected_content = Vec::with_capacity(output.content.len());
    for item in &output.content {
        match item {
            ToolOutputContent::Image {
                mime_type,
                data,
                detail,
            } => {
                image_parts.push(chat_image_part(mime_type, data, *detail));
                projected_content.push(ToolOutputContent::Text {
                    text: CHAT_TOOL_IMAGE_ATTACHED_MARKER.to_string(),
                });
            }
            ToolOutputContent::EmbeddedResource {
                mime_type: Some(mime_type),
                data: Some(data),
                ..
            } if mime_type.starts_with("image/") => {
                image_parts.push(chat_image_part(mime_type, data, None));
                projected_content.push(ToolOutputContent::Text {
                    text: CHAT_TOOL_IMAGE_ATTACHED_MARKER.to_string(),
                });
            }
            _ => projected_content.push(item.clone()),
        }
    }
    let projected = ToolOutput {
        content: projected_content,
        structured_content: output.structured_content.clone(),
        is_error: output.is_error,
    };
    Ok(ChatToolOutput {
        tool_content: legacy_text(&projected)?,
        image_parts,
    })
}

fn chat_image_part(mime_type: &str, data: &str, detail: Option<ToolOutputImageDetail>) -> Value {
    let mut image_url = json!({
        "url": format!("data:{mime_type};base64,{data}")
    });
    if let Some(detail) = detail {
        image_url["detail"] = Value::String(chat_image_detail_name(detail).to_string());
    }
    json!({
        "type": "image_url",
        "image_url": image_url
    })
}

fn chat_image_detail_name(detail: ToolOutputImageDetail) -> &'static str {
    match detail {
        ToolOutputImageDetail::Low => "low",
        ToolOutputImageDetail::High | ToolOutputImageDetail::Original => "high",
        ToolOutputImageDetail::Auto => "auto",
    }
}

pub(crate) fn responses_output(output: &ToolOutput) -> ModelResult<Value> {
    output
        .validate()
        .map_err(|error| ModelError::InvalidRequest(error.to_string()))?;
    if let Some(structured) = output.structured_content.as_ref().filter(|value| {
        output.content.len() == 1
            && matches!(&output.content[0], ToolOutputContent::Structured { value: content } if content == *value)
    }) {
        return serde_json::to_string(structured)
            .map(Value::String)
            .map_err(|_| ModelError::InvalidRequest("tool output could not be encoded".into()));
    }
    let mut items = Vec::with_capacity(output.content.len() + 1);
    for item in &output.content {
        items.push(match item {
            ToolOutputContent::Text { text } => json!({"type": "input_text", "text": text}),
            ToolOutputContent::Structured { value } => json!({
                "type": "input_text",
                "text": serde_json::to_string(value).map_err(|_| {
                    ModelError::InvalidRequest("structured tool output could not be encoded".into())
                })?
            }),
            ToolOutputContent::Image {
                mime_type,
                data,
                detail,
            } => {
                let mut value = json!({
                    "type": "input_image",
                    "image_url": format!("data:{mime_type};base64,{data}"),
                });
                if let Some(detail) = detail {
                    value["detail"] = Value::String(image_detail_name(*detail).to_string());
                }
                value
            }
            ToolOutputContent::Audio { mime_type, data } => json!({
                "type": "input_audio",
                "audio_url": format!("data:{mime_type};base64,{data}"),
            }),
            ToolOutputContent::ResourceLink {
                name,
                uri,
                mime_type,
            } => json!({
                "type": "input_text",
                "text": serde_json::to_string(&json!({
                    "type": "resource_link",
                    "name": name,
                    "uri": uri,
                    "mimeType": mime_type,
                })).expect("JSON values are serializable")
            }),
            ToolOutputContent::EmbeddedResource {
                uri,
                mime_type,
                text: Some(text),
                ..
            } => json!({
                "type": "input_text",
                "text": serde_json::to_string(&json!({
                    "type": "embedded_resource",
                    "uri": uri,
                    "mimeType": mime_type,
                    "text": text,
                })).expect("JSON values are serializable")
            }),
            ToolOutputContent::EmbeddedResource {
                uri,
                mime_type,
                data: Some(data),
                ..
            } => {
                if mime_type
                    .as_deref()
                    .is_some_and(|value| value.starts_with("image/"))
                {
                    json!({
                        "type": "input_image",
                        "image_url": format!(
                            "data:{};base64,{data}",
                            mime_type.as_deref().unwrap_or("image/octet-stream")
                        ),
                    })
                } else if mime_type
                    .as_deref()
                    .is_some_and(|value| value.starts_with("audio/"))
                {
                    json!({
                        "type": "input_audio",
                        "audio_url": format!(
                            "data:{};base64,{data}",
                            mime_type.as_deref().unwrap_or("audio/octet-stream")
                        ),
                    })
                } else {
                    json!({
                        "type": "input_text",
                        "text": serde_json::to_string(&json!({
                            "type": "embedded_resource",
                            "uri": uri,
                            "mimeType": mime_type,
                            "binaryDataOmitted": true,
                        })).expect("JSON values are serializable")
                    })
                }
            }
            ToolOutputContent::EmbeddedResource { uri, mime_type, .. } => json!({
                "type": "input_text",
                "text": serde_json::to_string(&json!({
                    "type": "embedded_resource",
                    "uri": uri,
                    "mimeType": mime_type,
                })).expect("JSON values are serializable")
            }),
        });
    }
    if let Some(structured) = output
        .structured_content
        .as_ref()
        .filter(|value| !value.is_null())
        .filter(|structured| {
            !output.content.iter().any(
                |item| matches!(item, ToolOutputContent::Structured { value } if value == *structured),
            )
        })
    {
        items.push(json!({
            "type": "input_text",
            "text": serde_json::to_string(structured).map_err(|_| {
                ModelError::InvalidRequest("structured tool output could not be encoded".into())
            })?
        }));
    }
    Ok(Value::Array(items))
}

pub(crate) fn anthropic_content(output: &ToolOutput) -> ModelResult<Vec<Value>> {
    output
        .validate()
        .map_err(|error| ModelError::InvalidRequest(error.to_string()))?;
    let mut items = Vec::with_capacity(output.content.len() + 1);
    for item in &output.content {
        items.push(match item {
            ToolOutputContent::Text { text } => json!({"type": "text", "text": text}),
            ToolOutputContent::Structured { value } => json!({
                "type": "text",
                "text": serde_json::to_string(value).map_err(|_| {
                    ModelError::InvalidRequest("structured tool output could not be encoded".into())
                })?
            }),
            ToolOutputContent::Image {
                mime_type, data, ..
            } => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": mime_type,
                    "data": data
                }
            }),
            ToolOutputContent::Audio { mime_type, .. } => json!({
                "type": "text",
                "text": serde_json::to_string(&json!({
                    "type": "unsupported_audio",
                    "mimeType": mime_type,
                    "message": "Anthropic Messages cannot receive audio tool output."
                })).expect("JSON values are serializable")
            }),
            ToolOutputContent::ResourceLink {
                name,
                uri,
                mime_type,
            } => json!({
                "type": "text",
                "text": serde_json::to_string(&json!({
                    "type": "resource_link",
                    "name": name,
                    "uri": uri,
                    "mimeType": mime_type,
                })).expect("JSON values are serializable")
            }),
            ToolOutputContent::EmbeddedResource {
                uri,
                mime_type,
                text: Some(text),
                ..
            } => json!({
                "type": "text",
                "text": serde_json::to_string(&json!({
                    "type": "embedded_resource",
                    "uri": uri,
                    "mimeType": mime_type,
                    "text": text,
                })).expect("JSON values are serializable")
            }),
            ToolOutputContent::EmbeddedResource {
                mime_type: Some(mime_type),
                data: Some(data),
                ..
            } if mime_type.starts_with("image/") => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": mime_type,
                    "data": data
                }
            }),
            ToolOutputContent::EmbeddedResource {
                uri,
                mime_type,
                data,
                ..
            } => json!({
                "type": "text",
                "text": serde_json::to_string(&json!({
                    "type": "embedded_resource",
                    "uri": uri,
                    "mimeType": mime_type,
                    "binaryDataOmitted": data.is_some(),
                })).expect("JSON values are serializable")
            }),
        });
    }
    if let Some(structured) = output
        .structured_content
        .as_ref()
        .filter(|value| !value.is_null())
        .filter(|structured| {
            !output.content.iter().any(
                |item| matches!(item, ToolOutputContent::Structured { value } if value == *structured),
            )
        })
    {
        items.push(json!({
            "type": "text",
            "text": serde_json::to_string(structured).map_err(|_| {
                ModelError::InvalidRequest("structured tool output could not be encoded".into())
            })?
        }));
    }
    Ok(items)
}

fn image_detail_name(detail: ToolOutputImageDetail) -> &'static str {
    match detail {
        ToolOutputImageDetail::Low => "low",
        ToolOutputImageDetail::High => "high",
        ToolOutputImageDetail::Auto => "auto",
        ToolOutputImageDetail::Original => "original",
    }
}
