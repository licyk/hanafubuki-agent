use std::collections::HashMap;

use super::AdvisorOptions;
use crate::conversation_reader::AgentConversationReader;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{
    flattened_tool_call_name, AgentError, AgentHistorySortDirection, AgentImageAttachmentRef,
    AgentSessionId, AgentTranscriptPageRequest, ConversationItem, AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};

#[derive(Clone, Default)]
pub(super) struct Evidence {
    pub text: String,
    pub images: Vec<AgentImageAttachmentRef>,
}

impl AdvisorOptions {
    pub(super) fn outbound(&self, text: String) -> String {
        if self.redact_secrets {
            hanafubuki_agent_memory::redact_memory_text(&text, &[])
        } else {
            text
        }
    }
}

pub(super) fn conversation(
    reader: &AgentConversationReader,
    id: AgentSessionId,
    opts: &AdvisorOptions,
    cancel: &CancellationToken,
) -> Result<Evidence, AgentError> {
    let mut groups = Vec::new();
    let mut images = Vec::new();
    let mut image_bytes = 0;
    let mut pending = HashMap::new();
    let mut remaining = opts.history_chars;
    let mut cursor = None;
    let mut revision = None;
    while remaining > 0 {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let page = reader.read_transcript_page(AgentTranscriptPageRequest {
            session_id: id,
            page_size: AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
            cursor,
            direction: AgentHistorySortDirection::Descending,
            through_revision: revision,
        })?;
        for item in page.items {
            let (text, attachments) = match item.content {
                ConversationItem::UserMessage { text, images }
                | ConversationItem::SteeringMessage { text, images, .. } => {
                    (format!("user: {text}"), images)
                }
                ConversationItem::AssistantMessage { text, .. } => {
                    (format!("assistant: {text}"), Vec::new())
                }
                ConversationItem::ToolOutput {
                    call_id,
                    output,
                    images,
                    ..
                } => {
                    pending.insert(call_id, (output, images));
                    continue;
                }
                ConversationItem::ToolCall {
                    call_id,
                    namespace,
                    name,
                    arguments,
                    ..
                } => {
                    let result = pending.remove(&call_id);
                    let name = flattened_tool_call_name(namespace.as_deref(), &name);
                    if opts.excluded_tools.iter().any(|n| n == name.as_ref())
                        || name == "ask_advisor"
                    {
                        continue;
                    }
                    let Some((output, images)) = result else {
                        continue;
                    };
                    if opts.summary_tools.iter().any(|n| n == name.as_ref()) {
                        (
                            format!(
                                "{name}: {} ({} result parts, {} images)",
                                if output.is_error {
                                    "failed"
                                } else {
                                    "completed"
                                },
                                output.content.len(),
                                images.len()
                            ),
                            Vec::new(),
                        )
                    } else {
                        (
                            format!("Tool {name} {arguments}\nResult: {}", result_text(&output)),
                            images,
                        )
                    }
                }
                _ => continue,
            };
            let mut text = opts.outbound(text);
            for image in attachments {
                if opts.share_images
                    && images.len() < 4
                    && image.byte_length <= 4 * 1024 * 1024
                    && image_bytes + image.byte_length <= 8 * 1024 * 1024
                {
                    image_bytes += image.byte_length;
                    images.push(image);
                } else {
                    text.push_str("\n[Image withheld]");
                }
            }
            if let Some((i, _)) = text.char_indices().nth(remaining) {
                text.truncate(i)
            }
            remaining = remaining.saturating_sub(text.chars().count() + 1);
            groups.push(text);
            if remaining == 0 {
                break;
            }
        }
        cursor = page.older_cursor;
        if cursor.is_none() {
            break;
        }
        revision = Some(page.through_revision);
    }
    groups.reverse();
    Ok(Evidence {
        text: groups.join("\n"),
        images,
    })
}

fn result_text(output: &ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|part| match part {
            ToolOutputContent::Text { text } => Some(text.clone()),
            ToolOutputContent::Structured { value } => Some(value.to_string()),
            ToolOutputContent::ResourceLink { name, .. } => Some(name.clone()),
            ToolOutputContent::EmbeddedResource { text, .. } => text.clone(),
            ToolOutputContent::Image { .. } | ToolOutputContent::Audio { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
