use sha2::{Digest, Sha256};

use super::*;

mod admission;
mod commit;
mod execute;
mod prepare;

impl Drop for CompactionExecutionGuard {
    fn drop(&mut self) {
        if self.armed {
            self.service.finish_compaction_terminal_on_drop(
                self.identity,
                AgentCompactionStatus::Cancelled,
            );
        }
    }
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
            | ModelInputItem::CompactionTrigger => {}
        }
    }
    Ok(filtered)
}

async fn collect_v2_compaction(
    plan: &CompactionPlan,
) -> Result<
    (
        hanafubuki_agent_model_api::ModelOpaqueCompactionPayload,
        Option<hanafubuki_agent_model_api::ModelUsage>,
    ),
    AgentError,
> {
    let mut stream = plan
        .model
        .stream(plan.request.clone(), plan.cancellation.clone())
        .await
        .map_err(map_model_error)?;
    let mut response_id = None;
    let mut payload = None;
    while let Some(event) = stream.next().await {
        match event.map_err(map_model_error)? {
            ModelEvent::ResponseStarted {
                response_id: id, ..
            } => {
                if response_id.replace(id).is_some() {
                    return Err(AgentError::Protocol(
                        "v2 compaction started multiple responses".to_string(),
                    ));
                }
            }
            ModelEvent::OutputItemAdded { .. } | ModelEvent::ReasoningSummaryDelta { .. } => {
                require_response_started(&response_id)?;
            }
            ModelEvent::AssistantTextDelta { .. } => {
                return Err(AgentError::Protocol(
                    "v2 compaction returned assistant text".to_string(),
                ));
            }
            ModelEvent::OutputItemCompleted { item } => match item {
                ModelOutputItem::Compaction { payload: value, .. } if payload.is_none() => {
                    payload = Some(value);
                }
                ModelOutputItem::Compaction { .. } => {
                    return Err(AgentError::Protocol(
                        "v2 compaction returned multiple compaction items".to_string(),
                    ));
                }
                _ => {
                    return Err(AgentError::Protocol(
                        "v2 compaction returned a non-compaction output item".to_string(),
                    ));
                }
            },
            ModelEvent::ResponseIncomplete { reason, .. } => {
                return Err(AgentError::Protocol(format!(
                    "v2 compaction response was incomplete: {reason}"
                )));
            }
            ModelEvent::ResponseCompleted {
                response_id: id,
                usage,
                end_turn,
            } => {
                if require_response_started(&response_id)? != &id || end_turn == Some(false) {
                    return Err(AgentError::Protocol(
                        "v2 compaction response completion is inconsistent".to_string(),
                    ));
                }
                let payload = payload.ok_or_else(|| {
                    AgentError::Protocol("v2 compaction returned no compaction item".to_string())
                })?;
                if stream.next().await.is_some() {
                    return Err(AgentError::Protocol(
                        "v2 compaction returned an event after response completion".to_string(),
                    ));
                }
                return Ok((payload, usage));
            }
        }
    }
    Err(AgentError::Protocol(
        "v2 compaction stream ended before completion".to_string(),
    ))
}

fn allocate_replacement_history(
    record: &mut SessionRecord,
    replacement: &[ConversationItem],
) -> Vec<AgentHistoryItem> {
    replacement
        .iter()
        .map(|content| {
            let item_id = AgentHistoryItemId::new(record.conversation.next_history_item_id);
            record.conversation.next_history_item_id = record
                .conversation
                .next_history_item_id
                .checked_add(1)
                .expect("Agent history item id exhausted");
            AgentHistoryItem {
                source_session_id: record.session_id,
                item_id,
                item_revision: 0,
                turn_id: None,
                logical_segment: 0,
                logical_order: item_id.value(),
                turn_working_millis: None,
                content: content.clone(),
            }
        })
        .collect()
}

fn canonical_initial_context(record: &SessionRecord) -> ConversationItem {
    let mut content = String::new();
    content.push_str("Mode: ");
    content.push_str(match record.settings.config.mode {
        crate::AgentSessionMode::Default => "default",
        crate::AgentSessionMode::Plan => "plan",
    });
    content.push('\n');
    if let Some(task_plan) = record.runtime.task_plan.as_ref() {
        content.push_str("\nCurrent task checklist:\n");
        if let Ok(encoded) = serde_json::to_string(task_plan) {
            content.push_str(&encoded);
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(record.settings.config.instructions.as_bytes());
    hasher.update(content.as_bytes());
    ConversationItem::CanonicalInitialContext {
        digest: hasher.finalize().into(),
        content,
    }
}

fn canonical_context_digest(item: &ConversationItem) -> [u8; 32] {
    match item {
        ConversationItem::CanonicalInitialContext { digest, .. } => *digest,
        // Every caller passes the plan's item, built by
        // `canonical_initial_context`, which returns only this variant.
        _ => unreachable!("canonical context helper returned the wrong item"),
    }
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

fn build_compaction_input(
    source: &[ConversationItem],
    images: &LoadedImages,
) -> Result<(Vec<ModelInputItem>, Vec<usize>), AgentError> {
    let mut input = Vec::new();
    let mut trim_prefix_ends = Vec::new();
    for item in source {
        if matches!(item, ConversationItem::TurnStart { .. }) && !input.is_empty() {
            trim_prefix_ends.push(input.len());
        }
        if let Some(model) = item.as_model_input_resolved(|image| images.bytes(image))? {
            input.push(model);
        }
    }
    Ok((input, trim_prefix_ends))
}

fn plan_retry_limit(plan: &CompactionPlan) -> u32 {
    plan.sampling_retry_policy.maximum_retries
}

async fn wait_for_compaction_retry(
    plan: &CompactionPlan,
    current_retry: u32,
) -> Result<(), AgentError> {
    let delay = plan.sampling_retry_policy.delay_for_retry(current_retry);
    if super::retry_wait::cancellable_delay(&plan.cancellation, delay).await {
        Err(AgentError::Cancelled)
    } else {
        Ok(())
    }
}

fn is_context_window_exceeded(error: &ModelError) -> bool {
    matches!(error, ModelError::ContextWindowExceeded { .. })
}

fn trim_local_compaction_request(
    request: &mut ModelRequest,
    prefix_ends: &[usize],
    trim_index: &mut usize,
    removed_inputs: &mut usize,
) -> Result<(), AgentError> {
    let target_end = prefix_ends.get(*trim_index).copied().ok_or_else(|| {
        AgentError::Protocol(
            "local compaction exceeded the provider context window after all safe history groups were trimmed"
                .to_string(),
        )
    })?;
    let remove = target_end.saturating_sub(*removed_inputs);
    if remove == 0 || remove >= request.input.len().saturating_sub(1) {
        return Err(AgentError::Protocol(
            "local compaction could not trim another complete history group".to_string(),
        ));
    }
    request.input.drain(..remove);
    *removed_inputs = target_end;
    *trim_index = trim_index.saturating_add(1);
    Ok(())
}

fn validate_compaction_summary(summary: &str, limits: AgentHardLimits) -> Result<(), AgentError> {
    if summary.trim().is_empty() {
        return Err(AgentError::Protocol(
            "compaction summary must not be empty".to_string(),
        ));
    }
    enforce_size(
        "compaction summary bytes",
        summary.len(),
        limits.maximum_compaction_summary_bytes,
    )?;
    let tokens = hanafubuki_agent_contracts::estimate_tokens(summary.len());
    if tokens > limits.maximum_compaction_summary_tokens {
        return Err(AgentError::LimitExceeded {
            resource: "compaction summary tokens",
            limit: limits.maximum_compaction_summary_tokens as usize,
        });
    }
    Ok(())
}

fn compaction_usage(sample: &CompactionSample) -> AgentCompactionUsage {
    match sample.reported_usage {
        Some(usage) => AgentCompactionUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            source: AgentUsageSource::ProviderReported,
        },
        None => AgentCompactionUsage {
            input_tokens: sample.estimated_input_tokens,
            output_tokens: sample.estimated_output_tokens,
            source: AgentUsageSource::Estimated,
        },
    }
}
