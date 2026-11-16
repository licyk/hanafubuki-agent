//! Resolve stored image references only while constructing model input.

use super::conversation_item::model_image_detail;
use super::{AgentImageAttachmentRef, ConversationItem};
use base64::Engine as _;

impl ConversationItem {
    /// The model input of this item with its image attachments' bytes from
    /// `image` (`image_attachments` names the ones it asks for).
    pub(crate) fn as_model_input_resolved(
        &self,
        mut image: impl FnMut(&AgentImageAttachmentRef) -> Result<Vec<u8>, super::AgentError>,
    ) -> Result<Option<hanafubuki_agent_model_api::ModelInputItem>, super::AgentError> {
        match self {
            Self::UserMessage { text, images } | Self::SteeringMessage { text, images, .. } => {
                let mut content = Vec::with_capacity(images.len().saturating_add(1));
                if !text.is_empty() {
                    content.push(hanafubuki_agent_model_api::ModelUserContent::InputText {
                        text: text.clone(),
                    });
                }
                for attachment in images {
                    content.push(hanafubuki_agent_model_api::ModelUserContent::InputImage {
                        mime_type: attachment.mime_type.clone(),
                        data: image(attachment)?,
                        detail: model_image_detail(attachment.detail),
                    });
                }
                Ok(Some(
                    hanafubuki_agent_model_api::ModelInputItem::UserMessage { content },
                ))
            }
            Self::ToolOutput {
                call_id,
                origin,
                output,
                images,
                ..
            } if !images.is_empty() => {
                let mut output = output.clone();
                output.content.reserve(images.len());
                for attachment in images {
                    let bytes = image(attachment)?;
                    if let Some(index) = attachment.tool_output_index {
                        use hanafubuki_agent_tool_contracts::ToolOutputContent;
                        let data = match output.content.get_mut(index) {
                            Some(ToolOutputContent::Image { data, .. }) => data,
                            Some(ToolOutputContent::EmbeddedResource {
                                data: Some(data), ..
                            }) => data,
                            _ => {
                                return Err(super::AgentError::Persistence(
                                    "stored tool image has no matching content slot".into(),
                                ))
                            }
                        };
                        *data = base64::engine::general_purpose::STANDARD.encode(bytes);
                        continue;
                    }
                    output.content.push(
                        hanafubuki_agent_tool_contracts::ToolOutputContent::Image {
                            mime_type: attachment.mime_type.clone(),
                            data: base64::engine::general_purpose::STANDARD.encode(bytes),
                            detail: Some(attachment.detail.into()),
                        },
                    );
                }
                Ok(Some(match origin {
                    hanafubuki_agent_model_api::ModelToolCallOrigin::Freeform => {
                        hanafubuki_agent_model_api::ModelInputItem::CustomToolOutput {
                            call_id: call_id.clone(),
                            output,
                        }
                    }
                    _ => hanafubuki_agent_model_api::ModelInputItem::ToolOutput {
                        call_id: call_id.clone(),
                        output,
                    },
                }))
            }
            _ => Ok(self.as_model_input()),
        }
    }
}
