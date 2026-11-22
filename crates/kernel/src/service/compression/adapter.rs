//! The history as the compression crate's message log, and its view back as
//! history items.

use std::borrow::Cow;
use std::collections::HashMap;

use hanafubuki_agent_compression::{is_rendered_summary, Message, Role};
use hanafubuki_agent_model_api::{GeminiReplayPart, ModelReasoningReplay};
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
        let tool_names: HashMap<&str, Cow<'_, str>> = items
            .iter()
            .filter_map(|item| match &item.content {
                ConversationItem::ToolCall {
                    call_id,
                    namespace,
                    name,
                    ..
                } => Some((
                    call_id.as_str(),
                    crate::flattened_tool_call_name(namespace.as_deref(), name),
                )),
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
    /// each summary it rendered at its place, as a summary item. Reasoning
    /// stays with the message after it: before it, after the summaries
    /// placed before it, and out of the view with it, since it is sent back
    /// on that message. A GenerateContent layout instead follows the model
    /// response whose parts it describes.
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
        let mut layouts = self.projected_layouts(view, &position, &originals);
        let mut projected = Vec::with_capacity(self.items.len());
        let mut reasoning = Vec::new();
        let mut next = 0;
        for (index, (item, id)) in self.items.iter().zip(&self.ids).enumerate() {
            if matches!(&item.content, ConversationItem::Reasoning { replay: Some(replay), .. } if replay.is_part_layout())
            {
                projected.extend(layouts.remove(&index));
                continue;
            }
            let Some(id) = id else {
                if matches!(item.content, ConversationItem::Reasoning { .. }) {
                    reasoning.push(item.content.clone());
                } else {
                    projected.append(&mut reasoning);
                    projected.push(item.content.clone());
                }
                continue;
            };
            let Some(&at) = position.get(id.as_str()) else {
                reasoning.clear();
                continue;
            };
            if at >= next {
                projected.extend(view[next..at].iter().filter_map(summary_item));
                next = at + 1;
            }
            projected.append(&mut reasoning);
            let shown = &view[at];
            projected.push(match originals.get(id.as_str()) {
                Some(original) if original.text != shown.text => {
                    with_view_text(&item.content, &shown.text)
                }
                _ => item.content.clone(),
            });
            projected.extend(layouts.remove(&index));
        }
        projected.append(&mut reasoning);
        projected.extend(view[next.min(view.len())..].iter().filter_map(summary_item));
        projected
    }

    /// GenerateContent's trailing layout describes the entire preceding model
    /// response. A partial view can still echo an unchanged call's signature,
    /// but must take its visible text from the view.
    fn projected_layouts(
        &self,
        view: &[Message],
        position: &HashMap<&str, usize>,
        originals: &HashMap<&str, &Message>,
    ) -> HashMap<usize, ConversationItem> {
        let mut layouts = HashMap::new();
        let mut start = 0;
        for (index, item) in self.items.iter().enumerate() {
            let (route, parts, step) = match &item.content {
                ConversationItem::Reasoning {
                    sampling_step,
                    replay: Some(ModelReasoningReplay::GeminiGenerate { route, parts }),
                    ..
                } => (route, parts, *sampling_step),
                ConversationItem::AssistantMessage { .. }
                | ConversationItem::ToolCall { .. }
                | ConversationItem::Reasoning { .. } => continue,
                _ => {
                    start = index + 1;
                    continue;
                }
            };
            let mut previous = None;
            let mut intact = true;
            for id in self.ids[start..index].iter().flatten() {
                let Some(&at) = position.get(id.as_str()) else {
                    intact = false;
                    continue;
                };
                intact &= previous.is_none_or(|last| last + 1 == at)
                    && view[at].text == originals[id.as_str()].text;
                previous = Some(at);
            }
            if previous.is_some() && intact {
                layouts.insert(index, item.content.clone());
            } else if previous.is_some() {
                for at in start..index {
                    let ConversationItem::ToolCall { call_id, .. } = &self.items[at].content else {
                        continue;
                    };
                    let Some(id) = self.ids[at].as_deref() else {
                        continue;
                    };
                    let Some(&shown) = position.get(id) else {
                        continue;
                    };
                    let binding = parts.iter().find(|part| {
                        matches!(part, GeminiReplayPart::Call { call_id: id, .. } if id == call_id)
                    });
                    if let Some(GeminiReplayPart::Call {
                        provider_id,
                        signature,
                        ..
                    }) = binding
                    {
                        let same_args = view[shown].text == originals[id].text;
                        let replay = ModelReasoningReplay::GeminiGenerate {
                            route: route.clone(),
                            parts: vec![GeminiReplayPart::Call {
                                call_id: call_id.clone(),
                                provider_id: provider_id.clone(),
                                signature: if same_args { signature.clone() } else { None },
                            }],
                        };
                        layouts.insert(
                            at,
                            ConversationItem::Reasoning {
                                text: String::new(),
                                duration_millis: 0,
                                sampling_step: step,
                                truncated: false,
                                replay: Some(replay),
                            },
                        );
                    }
                }
            }
            start = index + 1;
        }
        layouts
    }
}

/// The id of `item`'s message: the item's own identity, unique within the
/// session's lineage.
fn message_id(item: &AgentHistoryItem) -> String {
    format!("{}:{}", item.source_session_id, item.item_id.value())
}

fn item_message(
    item: &AgentHistoryItem,
    tool_names: &HashMap<&str, Cow<'_, str>>,
) -> Option<Message> {
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
            namespace,
            name,
            arguments,
            ..
        } => Message::tool_call(id, call_id.clone(), crate::flattened_tool_call_name(namespace.as_deref(), name).into_owned(), arguments.to_string()),
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
                    .map(|name| name.as_ref())
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
        | ConversationItem::PromptContext { .. }
        | ConversationItem::SkillContext { .. }
        | ConversationItem::Reasoning { .. }
        | ConversationItem::ToolSelection { .. }
        | ConversationItem::PermissionReview { .. }
        | ConversationItem::ContinuationContext { .. }
        // What the person ran stays as it is, as the person's messages do,
        // and so do the background work's reports.
        | ConversationItem::UserShellCommand { .. }
        | ConversationItem::WakeEvents { .. } => return None,
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

#[cfg(test)]
mod tests;
