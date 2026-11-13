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
    let (request_without_media_bytes, images) = without_media(request)?;
    estimate_tokens(serialized_len(&request_without_media_bytes)?)
        .checked_add(images.tokens)
        .ok_or(AgentError::UsageOverflow {
            field: "estimated model request tokens",
        })
}

/// The images a request carries, and their estimate.
#[derive(Default)]
pub(super) struct ImageEstimate {
    pub(super) count: u64,
    pub(super) tokens: u64,
}

/// `request` with its media payloads cleared, which the bytes estimate
/// leaves to the images' own, and that estimate.
pub(super) fn without_media(
    request: &ModelRequest,
) -> Result<(ModelRequest, ImageEstimate), AgentError> {
    let mut request_without_media_bytes = request.clone();
    let mut images = ImageEstimate::default();
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
                        &mut images,
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
                                &mut images,
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
                                &mut images,
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
    Ok((request_without_media_bytes, images))
}

/// The bytes of `value` serialized, which the estimate counts.
pub(super) fn serialized_len(value: &impl serde::Serialize) -> Result<usize, AgentError> {
    serde_json::to_vec(value)
        .map(|encoded| encoded.len())
        .map_err(|error| AgentError::Protocol(format!("estimate model request: {error}")))
}

fn add_estimated_image_tokens(
    images: &mut ImageEstimate,
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
    images.tokens = images
        .tokens
        .checked_add(tokens)
        .ok_or(AgentError::UsageOverflow {
            field: "image input tokens",
        })?;
    images.count = images.count.saturating_add(1);
    Ok(())
}

/// The tokens `items` are estimated at, by their retained bytes. A
/// reasoning item's text is never model input, so it is no context; only
/// the reasoning sent back from every turn (`reasoning_content`) counts, by
/// the bytes it sends back.
pub(in crate::service) fn estimate_conversation_items_tokens<'a>(
    items: impl IntoIterator<Item = &'a ConversationItem>,
) -> Result<u64, AgentError> {
    let bytes = items.into_iter().try_fold(0usize, |total, item| {
        total
            .checked_add(context_bytes(item))
            .ok_or(AgentError::UsageOverflow {
                field: "estimated conversation bytes",
            })
    })?;
    Ok(estimate_tokens(bytes))
}

fn context_bytes(item: &ConversationItem) -> usize {
    match item {
        ConversationItem::Reasoning { replay, .. } => replay
            .as_ref()
            .filter(|replay| replay.outlives_its_turn())
            .map_or(0, |replay| replay.text_len()),
        item => item.retained_bytes(),
    }
}
