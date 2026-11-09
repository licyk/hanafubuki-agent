//! The kernel's token estimates of model requests and history, on the one
//! bytes-per-token estimate (`hanafubuki_agent_contracts::estimate_tokens`).

use hanafubuki_agent_contracts::estimate_tokens;

use super::*;

/// The tokens `request` is estimated at: its serialized bytes without media
/// payloads, plus a per-image estimate. Every check of a request against the
/// context window counts with this, the stream included.
pub(in crate::service) fn estimate_model_request_tokens(
    request: &ModelRequest,
) -> Result<u64, AgentError> {
    let mut request_without_media_bytes = request.clone();
    let mut image_tokens = 0u64;
    for item in &mut request_without_media_bytes.input {
        match item {
            ModelInputItem::UserMessage { content } => {
                for part in content {
                    let hanafubuki_agent_model_api::ModelUserContent::InputImage {
                        data,
                        detail,
                        ..
                    } = part
                    else {
                        continue;
                    };
                    add_estimated_image_tokens(
                        &mut image_tokens,
                        data.len(),
                        matches!(detail, hanafubuki_agent_model_api::ModelImageDetail::Low),
                    )?;
                    data.clear();
                }
            }
            ModelInputItem::ToolOutput { output, .. }
            | ModelInputItem::CustomToolOutput { output, .. } => {
                for part in &mut output.content {
                    match part {
                        hanafubuki_agent_tool_contracts::ToolOutputContent::Image {
                            data,
                            detail,
                            ..
                        } => {
                            add_estimated_image_tokens(
                                &mut image_tokens,
                                base64::decoded_len_estimate(data.len()),
                                matches!(
                                    detail,
                                    Some(
                                        hanafubuki_agent_tool_contracts::ToolOutputImageDetail::Low
                                    )
                                ),
                            )?;
                            data.clear();
                        }
                        hanafubuki_agent_tool_contracts::ToolOutputContent::EmbeddedResource {
                            mime_type: Some(mime_type),
                            data: Some(data),
                            ..
                        } if mime_type.starts_with("image/") => {
                            add_estimated_image_tokens(
                                &mut image_tokens,
                                base64::decoded_len_estimate(data.len()),
                                false,
                            )?;
                            data.clear();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let bytes = serde_json::to_vec(&request_without_media_bytes)
        .map_err(|error| AgentError::Protocol(format!("estimate model request: {error}")))?
        .len();
    estimate_tokens(bytes)
        .checked_add(image_tokens)
        .ok_or(AgentError::UsageOverflow {
            field: "estimated model request tokens",
        })
}

fn add_estimated_image_tokens(
    total: &mut u64,
    byte_length: usize,
    low_detail: bool,
) -> Result<(), AgentError> {
    let tiles =
        u64::try_from(byte_length.saturating_add(256 * 1024 - 1) / (256 * 1024)).map_err(|_| {
            AgentError::UsageOverflow {
                field: "image token tiles",
            }
        })?;
    let tokens = if low_detail {
        85
    } else {
        85u64.saturating_add(170u64.saturating_mul(tiles.max(1)))
    };
    *total = total.checked_add(tokens).ok_or(AgentError::UsageOverflow {
        field: "image input tokens",
    })?;
    Ok(())
}

/// The tokens `items` are estimated at, by their retained bytes. Reasoning
/// is never model input, so it is no context.
pub(in crate::service) fn estimate_conversation_items_tokens(
    items: &[ConversationItem],
) -> Result<u64, AgentError> {
    let mut context = items
        .iter()
        .filter(|item| !matches!(item, ConversationItem::Reasoning { .. }));
    let bytes = context.try_fold(0usize, |total, item| {
        total
            .checked_add(item.retained_bytes())
            .ok_or(AgentError::UsageOverflow {
                field: "estimated conversation bytes",
            })
    })?;
    Ok(estimate_tokens(bytes))
}
