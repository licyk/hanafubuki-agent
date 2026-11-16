use base64::Engine as _;
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};

use super::*;

impl AgentService {
    /// Keep tool image bytes out of the canonical history and its projections.
    /// Content slots remain in place so resource metadata and image detail survive replay.
    pub(super) fn stage_tool_output_images(
        &self,
        session_id: AgentSessionId,
        output: &mut ToolOutput,
    ) -> Result<Vec<crate::AgentImageAttachmentRef>, AgentError> {
        let mut images = Vec::new();
        for (index, content) in output.content.iter_mut().enumerate() {
            let (mime_type, data) = match content {
                ToolOutputContent::Image {
                    mime_type, data, ..
                } => (mime_type, data),
                ToolOutputContent::EmbeddedResource {
                    mime_type: Some(mime_type),
                    data: Some(data),
                    ..
                } if mime_type.starts_with("image/") => (mime_type, data),
                _ => continue,
            };
            // Tool contracts accept unpadded base64 too; padding is transport syntax, not pixels.
            let bytes = base64::engine::GeneralPurpose::new(
                &base64::alphabet::STANDARD,
                base64::engine::GeneralPurposeConfig::new()
                    .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
            )
            .decode(data.as_bytes())
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
            let mut attachment = self
                .inner
                .history_store
                .stage_tool_image_attachment(session_id, bytes, mime_type.clone())
                .map_err(history_store_error)?;
            attachment.tool_output_index = Some(index);
            images.push(attachment);
            data.clear();
        }
        Ok(images)
    }
}

#[cfg(test)]
mod tests;
