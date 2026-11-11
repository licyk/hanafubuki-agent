use super::*;

/// The history a compaction installs: the compacted prefix (the world-state
/// checkpoint, then the replacement), which takes fresh ids, and the suffix
/// after it, which keeps its own.
pub(super) struct CompactedHistory {
    pub(super) prefix: Vec<ConversationItem>,
    pub(super) suffix: Vec<AgentHistoryItem>,
}

impl CompactedHistory {
    /// The installed history's contents, the next window's model context.
    pub(super) fn contents(&self) -> Vec<ConversationItem> {
        let mut contents = self.prefix.clone();
        contents.extend(self.suffix.iter().map(|item| item.content.clone()));
        contents
    }
}

pub(super) fn compacted_history<T: std::borrow::Borrow<AgentHistoryItem>>(
    sample: &CompactionSample,
    world_state_checkpoint: Option<ConversationItem>,
    suffix: &[T],
) -> CompactedHistory {
    CompactedHistory {
        prefix: world_state_checkpoint
            .into_iter()
            .chain(sample.replacement_history.iter().cloned())
            .collect(),
        suffix: suffix.iter().map(|item| item.borrow().clone()).collect(),
    }
}

/// A local compaction's replacement: the newest user messages, then the
/// summary.
pub(super) fn local_summary_replacement(
    plan: &CompactionPlan,
    summary: &str,
) -> Vec<ConversationItem> {
    let mut replacement =
        collect_real_user_messages(&plan.source_items, COMPACT_USER_MESSAGE_MAX_TOKENS);
    replacement.push(ConversationItem::CompactionSummary {
        summary: summary.to_string(),
        source_revision: plan.source_revision,
        compacted_item_count: plan.source_items.len(),
    });
    inject_initial_context(
        &mut replacement,
        plan.phase,
        &plan.canonical_initial_context,
    );
    replacement
}

/// A Responses compaction's replacement: the messages it kept that the
/// compacted history holds, and its compaction item.
pub(super) fn remote_replacement(
    plan: &CompactionPlan,
    replacement: &[ModelInputItem],
) -> Result<Vec<ConversationItem>, AgentError> {
    let mut replacement = filter_remote_replacement(replacement, &plan.source_items, &plan.images)?;
    if replacement.is_empty() {
        return Err(AgentError::Protocol(
            "remote compaction returned an empty replacement history".to_string(),
        ));
    }
    inject_initial_context(
        &mut replacement,
        plan.phase,
        &plan.canonical_initial_context,
    );
    Ok(replacement)
}

/// A v2 compaction's replacement: the newest user messages, then the
/// provider's payload.
pub(super) fn remote_v2_replacement(
    plan: &CompactionPlan,
    payload: hanafubuki_agent_model_api::ModelOpaqueCompactionPayload,
) -> Vec<ConversationItem> {
    let mut replacement =
        collect_real_user_messages(&plan.source_items, REMOTE_V2_RETAINED_MESSAGE_TOKEN_BUDGET);
    replacement.push(ConversationItem::ProviderCompaction { payload });
    inject_initial_context(
        &mut replacement,
        plan.phase,
        &plan.canonical_initial_context,
    );
    replacement
}

fn collect_real_user_messages(
    history: &[ConversationItem],
    maximum_tokens: u64,
) -> Vec<ConversationItem> {
    let candidates = history
        .iter()
        .filter_map(|item| match item {
            ConversationItem::UserMessage { text, images } => Some(ConversationItem::UserMessage {
                text: text.clone(),
                images: images.clone(),
            }),
            ConversationItem::SteeringMessage {
                sequence,
                text,
                images,
            } => Some(ConversationItem::SteeringMessage {
                sequence: *sequence,
                text: text.clone(),
                images: images.clone(),
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut remaining = maximum_tokens;
    let mut selected = Vec::new();
    for item in candidates.into_iter().rev() {
        if remaining == 0 {
            break;
        }
        let tokens = retained_user_message_tokens(&item).max(1);
        if tokens <= remaining {
            selected.push(item);
            remaining = remaining.saturating_sub(tokens);
        } else {
            selected.push(truncate_user_message(item, remaining));
            remaining = 0;
        }
    }
    selected.reverse();
    selected
}

fn retained_user_message_tokens(item: &ConversationItem) -> u64 {
    let (text, image_count) = match item {
        ConversationItem::UserMessage { text, images } => (text, images.len()),
        ConversationItem::SteeringMessage { text, images, .. } => (text, images.len()),
        _ => return 0,
    };
    hanafubuki_agent_contracts::estimate_tokens(text.len())
        .saturating_add(u64::try_from(image_count).unwrap_or(u64::MAX))
}

fn truncate_user_message(item: ConversationItem, maximum_tokens: u64) -> ConversationItem {
    match item {
        ConversationItem::UserMessage { text, images } => ConversationItem::UserMessage {
            text: truncate_user_text(&text, maximum_tokens),
            images,
        },
        ConversationItem::SteeringMessage {
            sequence,
            text,
            images,
        } => ConversationItem::SteeringMessage {
            sequence,
            text: truncate_user_text(&text, maximum_tokens),
            images,
        },
        // The only caller passes candidates filtered to these two variants.
        _ => unreachable!("only real user messages are retained"),
    }
}

fn truncate_user_text(text: &str, maximum_tokens: u64) -> String {
    let maximum_bytes = hanafubuki_agent_contracts::token_budget_bytes(maximum_tokens);
    if text.len() <= maximum_bytes {
        return text.to_string();
    }
    let mut end = maximum_bytes;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}\n[truncated]", &text[..end])
}

fn filter_remote_replacement(
    replacement: &[ModelInputItem],
    source: &[ConversationItem],
    images: &LoadedImages,
) -> Result<Vec<ConversationItem>, AgentError> {
    let resolved = source
        .iter()
        .map(|item| {
            item.as_model_input_resolved(|image| images.bytes(image))
                .map(|model| (item, model))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut filtered = Vec::new();
    for item in replacement {
        match item {
            ModelInputItem::UserMessage { .. } | ModelInputItem::AssistantMessage { .. } => {
                if let Some((source, _)) = resolved
                    .iter()
                    .find(|(_, model)| model.as_ref() == Some(item))
                {
                    filtered.push((*source).clone());
                }
            }
            ModelInputItem::Compaction { payload } => {
                filtered.push(ConversationItem::ProviderCompaction {
                    payload: payload.clone(),
                });
            }
            ModelInputItem::ContextFragment { .. }
            | ModelInputItem::ToolCall { .. }
            | ModelInputItem::CustomToolCall { .. }
            | ModelInputItem::ToolOutput { .. }
            | ModelInputItem::CustomToolOutput { .. }
            | ModelInputItem::ToolSearchCall { .. }
            | ModelInputItem::ToolSearchOutput { .. }
            | ModelInputItem::ContinuationContext { .. }
            | ModelInputItem::CompactionSummary { .. }
            | ModelInputItem::Reasoning { .. }
            | ModelInputItem::CompactionTrigger => {}
        }
    }
    Ok(filtered)
}

fn inject_initial_context(
    replacement: &mut Vec<ConversationItem>,
    phase: AgentCompactionPhase,
    canonical_initial_context: &ConversationItem,
) {
    if phase != AgentCompactionPhase::MidTurn {
        return;
    }
    let insertion = replacement
        .iter()
        .rposition(|item| {
            matches!(
                item,
                ConversationItem::UserMessage { .. } | ConversationItem::SteeringMessage { .. }
            )
        })
        .or_else(|| {
            replacement.iter().rposition(|item| {
                matches!(
                    item,
                    ConversationItem::CompactionSummary { .. }
                        | ConversationItem::ProviderCompaction { .. }
                )
            })
        })
        .unwrap_or(replacement.len());
    replacement.insert(insertion, canonical_initial_context.clone());
}
