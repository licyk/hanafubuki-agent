//! The history as the compression crate's message log, and its view back as
//! history items.

use std::collections::HashMap;

use hanafubuki_agent_compression::{is_rendered_summary, Message, Role};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};

use super::*;

/// The window's history as the crate's message log: one message per
/// message, call and output, each with its item's id. Every other item keeps
/// its place in the request around them. A compacted prefix (everything
/// before the window's first turn) and the summaries never fold.
pub(super) struct MessageLog<'a> {
    items: &'a [Arc<AgentHistoryItem>],
    /// Each item's message, if it has one.
    ids: Vec<Option<String>>,
    pub(super) messages: Vec<Message>,
}

impl<'a> MessageLog<'a> {
    pub(super) fn new(items: &'a [Arc<AgentHistoryItem>]) -> Self {
        let first_turn = items
            .iter()
            .position(|item| matches!(item.content, ConversationItem::TurnStart { .. }))
            .unwrap_or(items.len());
        let tool_names: HashMap<&str, &str> = items
            .iter()
            .filter_map(|item| match &item.content {
                ConversationItem::ToolCall { call_id, name, .. } => {
                    Some((call_id.as_str(), name.as_str()))
                }
                _ => None,
            })
            .collect();
        let mut ids = Vec::with_capacity(items.len());
        let mut messages = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let message = item_message(item, &tool_names).map(|mut message| {
                message.protected |= index < first_turn;
                message
            });
            ids.push(message.as_ref().map(|message| message.id.clone()));
            messages.extend(message);
        }
        Self {
            items,
            ids,
            messages,
        }
    }

    /// The images of each message that has any, by message id.
    pub(super) fn images_by_message(&self) -> HashMap<&str, &[crate::AgentImageAttachmentRef]> {
        self.items
            .iter()
            .zip(&self.ids)
            .filter_map(|(item, id)| {
                let images = item.content.image_attachments();
                (!images.is_empty()).then_some((id.as_deref()?, images))
            })
            .collect()
    }

    /// The items `view` keeps, in history order, with its texts (ref tags,
    /// compacted `compress` arguments, clipped outputs) written into them and
    /// each summary it rendered at its place, as a summary item.
    pub(super) fn apply_view(&self, view: &[Message]) -> Vec<ConversationItem> {
        let position: HashMap<&str, usize> = view
            .iter()
            .enumerate()
            .map(|(index, message)| (message.id.as_str(), index))
            .collect();
        let originals: HashMap<&str, &Message> = self
            .messages
            .iter()
            .map(|message| (message.id.as_str(), message))
            .collect();
        let mut projected = Vec::with_capacity(self.items.len());
        let mut next = 0;
        for (item, id) in self.items.iter().zip(&self.ids) {
            let Some(id) = id else {
                projected.push(item.content.clone());
                continue;
            };
            let Some(&at) = position.get(id.as_str()) else {
                continue;
            };
            if at >= next {
                projected.extend(view[next..at].iter().filter_map(summary_item));
                next = at + 1;
            }
            let shown = &view[at];
            projected.push(match originals.get(id.as_str()) {
                Some(original) if original.text != shown.text => {
                    with_view_text(&item.content, &shown.text)
                }
                _ => item.content.clone(),
            });
        }
        projected.extend(view[next.min(view.len())..].iter().filter_map(summary_item));
        projected
    }
}

/// The id of `item`'s message: the item's own identity, unique within the
/// session's lineage.
fn message_id(item: &AgentHistoryItem) -> String {
    format!("{}:{}", item.source_session_id, item.item_id.value())
}

fn item_message(item: &AgentHistoryItem, tool_names: &HashMap<&str, &str>) -> Option<Message> {
    let id = message_id(item);
    let image_handles = |images: &[crate::AgentImageAttachmentRef]| {
        images
            .iter()
            .map(|image| image.attachment_id.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    Some(match &item.content {
        ConversationItem::UserMessage { text, images }
        | ConversationItem::SteeringMessage { text, images, .. } => Message {
            images: image_handles(images),
            ..Message::text(id, Role::User, text.clone())
        },
        ConversationItem::AssistantMessage {
            text, model_text, ..
        } => Message::text(
            id,
            Role::Assistant,
            model_text.clone().unwrap_or_else(|| text.clone()),
        ),
        ConversationItem::ToolCall {
            call_id,
            name,
            arguments,
            ..
        } => Message::tool_call(id, call_id.clone(), name.clone(), arguments.to_string()),
        ConversationItem::ToolOutput {
            call_id,
            output,
            images,
            ..
        } => Message {
            images: image_handles(images),
            ..Message::tool_result(
                id,
                call_id.clone(),
                tool_names
                    .get(call_id.as_str())
                    .copied()
                    .unwrap_or_default(),
                output_text(output),
            )
        },
        ConversationItem::CompactionSummary { summary, .. } => Message {
            protected: true,
            ..Message::text(id, Role::User, summary.clone())
        },
        ConversationItem::ProviderCompaction { .. } => Message {
            protected: true,
            ..Message::text(id, Role::User, String::new())
        },
        ConversationItem::CanonicalInitialContext { .. }
        | ConversationItem::TurnStart { .. }
        | ConversationItem::WorldStateUpdate { .. }
        | ConversationItem::SkillContext { .. }
        | ConversationItem::Reasoning { .. }
        | ConversationItem::ToolSelection { .. }
        | ConversationItem::PermissionReview { .. }
        | ConversationItem::ContinuationContext { .. } => return None,
    })
}

/// A tool output's text as the model reads it: its text parts, structured
/// ones as JSON.
fn output_text(output: &ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|content| match content {
            ToolOutputContent::Text { text } => Some(text.clone()),
            ToolOutputContent::Structured { value } => Some(value.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `item` showing the view's text `shown` in place of its own.
fn with_view_text(item: &ConversationItem, shown: &str) -> ConversationItem {
    let mut item = item.clone();
    match &mut item {
        ConversationItem::UserMessage { text, .. }
        | ConversationItem::SteeringMessage { text, .. } => *text = shown.to_owned(),
        ConversationItem::AssistantMessage { model_text, .. } => {
            *model_text = Some(shown.to_owned())
        }
        // A live `compress` call with its arguments compacted; never tagged.
        ConversationItem::ToolCall { arguments, .. } => {
            if let Ok(compacted) = serde_json::from_str(shown) {
                *arguments = compacted;
            }
        }
        ConversationItem::ToolOutput { output, .. } => {
            // The text parts become the one text shown (a tag before the
            // original, or a clip of it); images and resources stay.
            output.content.retain(|content| {
                !matches!(
                    content,
                    ToolOutputContent::Text { .. } | ToolOutputContent::Structured { .. }
                )
            });
            output.content.insert(
                0,
                ToolOutputContent::Text {
                    text: shown.to_owned(),
                },
            );
            output.structured_content = None;
        }
        _ => {}
    }
    item
}

/// A summary the view rendered, as the summary item the model reads.
fn summary_item(message: &Message) -> Option<ConversationItem> {
    is_rendered_summary(message).then(|| ConversationItem::CompactionSummary {
        summary: message.text.clone(),
        source_revision: AgentRevision::new(0),
        compacted_item_count: 0,
    })
}
