use super::*;

mod commit;
mod estimate;
mod images;
mod prompt;
mod request;
mod stream;

pub(super) use estimate::{estimate_conversation_items_tokens, estimate_model_request_tokens};
pub(super) use images::{referenced_images, LoadedImages};
pub(super) use prompt::RequestContext;
use prompt::{ordered_input, prepare_prompt_step, request_context, PlacedInput};
pub(super) use request::SamplingStep;
pub(super) use stream::PendingReasoning;

const PLAN_MODE_INSTRUCTIONS: &str = include_str!("../../prompts/plan.md");

pub(super) fn next_model_usage(
    record: &SessionRecord,
    reported: Option<hanafubuki_agent_model_api::ModelUsage>,
    estimated_input_tokens: u64,
    estimated_output_tokens: u64,
) -> Result<Option<AgentModelUsage>, AgentError> {
    let (usage, source) = match reported {
        Some(usage) => (usage, AgentUsageSource::ProviderReported),
        None if record
            .runtime
            .usage
            .is_some_and(|usage| usage.source == AgentUsageSource::ProviderReported) =>
        {
            return Ok(None)
        }
        None => (
            hanafubuki_agent_model_api::ModelUsage {
                input_tokens: estimated_input_tokens,
                output_tokens: estimated_output_tokens,
                ..Default::default()
            },
            AgentUsageSource::Estimated,
        ),
    };
    let previous = record.runtime.usage.filter(|usage| usage.source == source);
    let add = |field, cumulative: fn(&AgentModelUsage) -> u64, value: u64| {
        previous
            .as_ref()
            .map_or(0, cumulative)
            .checked_add(value)
            .ok_or(AgentError::UsageOverflow { field })
    };
    let reports_cache = usage.cache_read_tokens.is_some() || usage.cache_write_tokens.is_some();
    Ok(Some(AgentModelUsage {
        last_response_input_tokens: usage.input_tokens,
        last_response_output_tokens: usage.output_tokens,
        cumulative_input_tokens: add(
            "cumulative input tokens",
            |usage| usage.cumulative_input_tokens,
            usage.input_tokens,
        )?,
        cumulative_output_tokens: add(
            "cumulative output tokens",
            |usage| usage.cumulative_output_tokens,
            usage.output_tokens,
        )?,
        context_window_tokens: record.settings.config.context_window_tokens,
        context_consumed_tokens: Some(usage.input_tokens),
        source,
        last_response_cache_read_tokens: usage.cache_read_tokens,
        last_response_cache_write_tokens: usage.cache_write_tokens,
        cumulative_cache_read_tokens: add(
            "cumulative cache read tokens",
            |usage| usage.cumulative_cache_read_tokens,
            usage.cache_read_tokens.unwrap_or(0),
        )?,
        cumulative_cache_write_tokens: add(
            "cumulative cache write tokens",
            |usage| usage.cumulative_cache_write_tokens,
            usage.cache_write_tokens.unwrap_or(0),
        )?,
        cache_reporting_responses: add(
            "cache reporting responses",
            |usage| usage.cache_reporting_responses,
            u64::from(reports_cache),
        )?,
    }))
}

pub(super) fn tool_omission_world_state_update(
    world_state: &mut BTreeMap<ContextSectionKind, AgentWorldStateSection>,
    catalog_revision: crate::AgentToolCatalogRevision,
    omissions: &[hanafubuki_agent_contracts::AgentToolOmission],
) -> Result<Option<ConversationItem>, AgentError> {
    if omissions.is_empty() {
        return Ok(crate::model::remove_world_state_section(
            world_state,
            ContextSectionKind::TOOL_CATALOG_OMISSIONS,
        ));
    }
    let payload = serde_json::json!({
        "catalogRevision": hex_digest(&catalog_revision.0),
        "omissions": omissions,
    });
    Ok(crate::model::replace_world_state_section(
        world_state,
        AgentWorldStateSection::from(hanafubuki_agent_contracts::ContextSection::from_json(
            ContextSectionKind::TOOL_CATALOG_OMISSIONS,
            &payload,
        )),
    ))
}

/// The request `record`'s next sampling step sends with `tool_plan` and
/// `prompt`, its images from `images`: the step itself, the admission
/// estimate before it and a compaction's check of the request after it all
/// build it here, so each counts the same tools, sections and memory.
pub(super) fn step_model_request(
    record: &SessionRecord,
    images: &LoadedImages,
    tool_plan: &AgentToolPlan,
    prompt: &RequestContext,
    maximum_tool_output_bytes: usize,
) -> Result<ModelRequest, AgentError> {
    step_model_request_projected(record, images, tool_plan, prompt, maximum_tool_output_bytes)
        .map(|(request, _)| request)
}

/// What building a request did with the session's compression.
#[derive(Default)]
pub(super) struct RequestCompression {
    /// The state after the projection, which only the step's own request
    /// persists.
    pub(super) state: Option<Arc<crate::AgentCompressionState>>,
    /// The content store, when the projection stored originals in it; kept
    /// like the state.
    pub(super) content_store: Option<Arc<crate::AgentContentStore>>,
    /// The token count the request was projected with, and whether it
    /// booked a user interrupt.
    pub(super) sent: Option<(u64, bool)>,
    /// A standard compaction is due (`hierarchical`'s terminal escape).
    pub(super) terminal_escape: bool,
    /// The algorithm panicked, with this message (bounded): the request
    /// carries the full history.
    pub(super) failed: Option<String>,
}

/// [`step_model_request`], with what its projection left.
pub(super) fn step_model_request_projected(
    record: &SessionRecord,
    images: &LoadedImages,
    tool_plan: &AgentToolPlan,
    prompt: &RequestContext,
    maximum_tool_output_bytes: usize,
) -> Result<(ModelRequest, RequestCompression), AgentError> {
    let (request, compression) = regular_model_request(
        record,
        images,
        tool_plan.model_specs(),
        tool_plan.parallel_tool_calls(),
        prompt,
        maximum_tool_output_bytes,
    )?;
    let request = if record.settings.effective_input_capabilities.image_input {
        request
    } else {
        hanafubuki_agent_model_api::sanitize_images_for_unsupported_input(&request).0
    };
    Ok((request, compression))
}

/// The request for `record`'s next sampling step, its images from `images`
/// (`StaleRevision` when the history references one `images` lacks).
fn regular_model_request(
    record: &SessionRecord,
    images: &LoadedImages,
    tools: Vec<hanafubuki_agent_tool_contracts::ToolSpec>,
    parallel_tool_calls: bool,
    context: &RequestContext,
    maximum_tool_output_bytes: usize,
) -> Result<(ModelRequest, RequestCompression), AgentError> {
    let latest_inline_context = record.conversation.history.iter().rposition(|item| {
        matches!(
            &item.content,
            ConversationItem::CanonicalInitialContext { .. }
        )
    });
    let latest_turn_start = record
        .conversation
        .history
        .iter()
        .rposition(|item| matches!(&item.content, ConversationItem::TurnStart { .. }));
    let use_inline_initial_context = latest_inline_context
        .is_some_and(|context| latest_turn_start.is_none_or(|turn| context > turn));
    // A compacted context window carries its own initial context (the mode
    // and the task checklist) until the next turn starts.
    let mut placed = context.fragments();
    let visible_items = record
        .conversation
        .history
        .iter()
        .filter(|item| {
            use_inline_initial_context
                || !matches!(
                    &item.content,
                    ConversationItem::CanonicalInitialContext { .. }
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    let (visible_history, trailing, compression, compression_instructions) =
        projected_history(record, &visible_items);
    let normalized_history = normalized_tool_pairs_for_model(&visible_history)?;
    let latest_world_state_updates = normalized_history
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            ConversationItem::WorldStateUpdate { updates } => {
                Some(updates.iter().map(move |update| {
                    let kind = match update {
                        AgentWorldStateUpdate::Replace { kind, .. }
                        | AgentWorldStateUpdate::Remove { kind, .. } => kind.clone(),
                    };
                    (kind, index)
                }))
            }
            _ => None,
        })
        .flatten()
        .collect::<BTreeMap<_, _>>();
    // A turn's reasoning blocks go back within it (after its `TurnStart`);
    // those that outlive their turn go back from every turn.
    let turn_start = normalized_history
        .iter()
        .rposition(|item| matches!(item, ConversationItem::TurnStart { .. }));
    for (index, item) in normalized_history.iter().enumerate() {
        if let ConversationItem::Reasoning { .. } = item {
            let in_current_turn = turn_start.is_some_and(|start| index > start);
            placed.extend(item.reasoning_input(in_current_turn).map(PlacedInput::Item));
            continue;
        }
        if let ConversationItem::WorldStateUpdate { updates } = item {
            for update in updates {
                let AgentWorldStateUpdate::Replace {
                    kind,
                    version,
                    content,
                } = update
                else {
                    continue;
                };
                if latest_world_state_updates.get(kind) == Some(&index) {
                    placed.extend(context.section(kind, Some(&hex_digest(version)), content));
                }
            }
            continue;
        }
        if let Some(item) = item.as_model_input_resolved(|image| images.bytes(image))? {
            placed.push(PlacedInput::Item(item));
        }
    }
    if !use_inline_initial_context {
        if let Some(task_plan) = record.runtime.task_plan.as_ref() {
            placed.push(PlacedInput::Item(task_plan_model_input(task_plan)));
        }
    }
    let mut input = ordered_input(placed);
    project_model_tool_outputs(
        &mut input,
        record.settings.config.context_window_tokens,
        maximum_tool_output_bytes,
    )?;
    input.extend(trailing.map(ModelInputItem::user_text));
    let mut instructions = record.settings.config.effective_instructions();
    if record.settings.config.mode == crate::AgentSessionMode::Plan {
        instructions = format!("{instructions}\n\n{PLAN_MODE_INSTRUCTIONS}");
    }
    if let Some(compression_instructions) = compression_instructions {
        instructions.push_str("\n\n");
        instructions.push_str(&compression_instructions);
    }
    let request = ModelRequest {
        model: record.settings.config.model.clone(),
        instructions,
        input,
        tools,
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::Auto,
        parallel_tool_calls,
        reasoning: record.settings.config.effective_reasoning(),
        output_format: None,
    };
    Ok((request, compression))
}

/// The history the request shows (projected while blocks or the strategy
/// call for it), its trailing nudge or status line, what the projection
/// left, and what `hierarchical` adds to the instructions.
fn projected_history(
    record: &SessionRecord,
    items: &[Arc<AgentHistoryItem>],
) -> (
    Arc<Vec<ConversationItem>>,
    Option<String>,
    RequestCompression,
    Option<String>,
) {
    use super::compression::{project_request, CompressionMode};
    let mode = CompressionMode::of(record);
    let full = || Arc::new(items.iter().map(|item| item.content.clone()).collect());
    if mode == CompressionMode::Off {
        return (full(), None, RequestCompression::default(), None);
    }
    match project_request(record, items, mode) {
        Ok(projection) => (
            projection.history,
            projection.trailing,
            RequestCompression {
                state: Some(projection.state),
                content_store: projection.content_store,
                sent: Some((projection.token_count, projection.abort_booked)),
                terminal_escape: projection.terminal_escape,
                failed: None,
            },
            projection.instructions,
        ),
        Err(message) => {
            let message = bounded_diagnostic(message);
            log::warn!(
                "Agent context compression failed; this turn sends the full history: {message}"
            );
            (
                full(),
                None,
                RequestCompression {
                    failed: Some(message),
                    ..RequestCompression::default()
                },
                None,
            )
        }
    }
}

fn hex_digest(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn project_model_tool_outputs(
    input: &mut [ModelInputItem],
    context_window_tokens: Option<u64>,
    maximum_tool_output_bytes: usize,
) -> Result<(), AgentError> {
    let Some(context_window_tokens) = context_window_tokens else {
        return Ok(());
    };
    let output_count = input
        .iter()
        .filter(|item| {
            matches!(
                item,
                ModelInputItem::ToolOutput { .. } | ModelInputItem::CustomToolOutput { .. }
            )
        })
        .count();
    if output_count == 0 {
        return Ok(());
    }
    // Reserve at most one quarter of the context window for all historical
    // tool results. At the conservative four-bytes-per-token estimate this is
    // one context-token byte per output set, divided fairly across results.
    let token_derived_total_bytes = usize::try_from(context_window_tokens).unwrap_or(usize::MAX);
    let per_output_bytes = token_derived_total_bytes
        .checked_div(output_count)
        .unwrap_or(0)
        .clamp(
            crate::tool_output::MINIMUM_CANONICAL_TOOL_OUTPUT_BYTES,
            maximum_tool_output_bytes,
        );
    for item in input {
        let output = match item {
            ModelInputItem::ToolOutput { output, .. }
            | ModelInputItem::CustomToolOutput { output, .. } => output,
            _ => continue,
        };
        let encoded = serde_json::to_vec(output)
            .map_err(|error| AgentError::Protocol(format!("project tool output: {error}")))?;
        let mut without_binary_payloads = output.clone();
        for content in &mut without_binary_payloads.content {
            match content {
                hanafubuki_agent_tool_contracts::ToolOutputContent::Image { data, .. }
                | hanafubuki_agent_tool_contracts::ToolOutputContent::Audio { data, .. } => {
                    data.clear()
                }
                hanafubuki_agent_tool_contracts::ToolOutputContent::EmbeddedResource {
                    data: Some(data),
                    ..
                } => data.clear(),
                _ => {}
            }
        }
        let projectable_bytes = serde_json::to_vec(&without_binary_payloads)
            .map_err(|error| AgentError::Protocol(format!("project tool output: {error}")))?
            .len();
        if projectable_bytes <= per_output_bytes {
            continue;
        }
        // Images, audio, and embedded binary resources are provider-native
        // modalities rather than prompt text. Preserve their already-bounded
        // canonical bytes and apply the context-derived allowance only to the
        // remaining textual/structured projection.
        let binary_payload_bytes = encoded.len().saturating_sub(projectable_bytes);
        let projection_limit = binary_payload_bytes
            .saturating_add(per_output_bytes)
            .min(maximum_tool_output_bytes);
        let (mut projected, truncation) =
            crate::tool_output::bound_tool_output(output.clone(), projection_limit)
                .map_err(|error| AgentError::Protocol(error.to_string()))?;
        if truncation.is_some() {
            for content in projected.content.iter_mut().rev() {
                let hanafubuki_agent_tool_contracts::ToolOutputContent::Text { text } = content
                else {
                    continue;
                };
                if text.starts_with("<hanafubuki-tool-output-truncation>") {
                    *text = text
                        .replacen(
                            "<hanafubuki-tool-output-truncation>",
                            "<hanafubuki-model-output-projected>",
                            1,
                        )
                        .replacen(
                            "</hanafubuki-tool-output-truncation>",
                            "</hanafubuki-model-output-projected>",
                            1,
                        );
                    break;
                }
            }
        }
        *output = projected;
    }
    Ok(())
}

pub(super) fn task_plan_model_input(plan: &AgentTaskPlanSnapshot) -> ModelInputItem {
    let mut content = String::new();
    if let Some(explanation) = plan
        .explanation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        content.push_str("Explanation: ");
        content.push_str(explanation);
        content.push('\n');
    }
    for step in &plan.plan {
        let status = match step.status {
            crate::AgentTaskPlanStepStatus::Pending => "pending",
            crate::AgentTaskPlanStepStatus::InProgress => "in_progress",
            crate::AgentTaskPlanStepStatus::Completed => "completed",
        };
        content.push_str("- [");
        content.push_str(status);
        content.push_str("] ");
        content.push_str(step.step.trim());
        content.push('\n');
    }
    ModelInputItem::ContextFragment {
        role: ModelContextRole::User,
        kind: "task_plan".to_string(),
        content: format!(
            "[Agent-maintained task checklist for the current turn. It is progress state only, not a user instruction, approval, ownership capability, or authorization.]\n{content}"
        ),
    }
}

pub(super) fn require_response_started(
    response_id: &Option<String>,
) -> Result<&String, AgentError> {
    response_id.as_ref().ok_or_else(|| {
        AgentError::Protocol("model emitted output before response start".to_string())
    })
}

fn complete_ignored_model_item(
    item_order: &[String],
    pending_items: &mut HashMap<String, PendingModelItem>,
    item_id: Option<&str>,
    item_type: &str,
) -> Result<(), AgentError> {
    let key = if let Some(item_id) = item_id {
        item_id.to_string()
    } else {
        item_order
            .iter()
            .find(|key| {
                matches!(
                    pending_items.get(*key),
                    Some(PendingModelItem::Ignored {
                        item_type: pending_type,
                        completed: false,
                    }) if pending_type == item_type
                )
            })
            .cloned()
            .ok_or_else(|| {
                AgentError::Protocol(format!(
                    "model completed an unknown anonymous {item_type} item"
                ))
            })?
    };
    match pending_items.get_mut(&key) {
        Some(PendingModelItem::Ignored {
            item_type: pending_type,
            completed,
        }) if pending_type == item_type && !*completed => {
            *completed = true;
            Ok(())
        }
        Some(PendingModelItem::Ignored {
            completed: true, ..
        }) => Err(AgentError::Protocol(format!(
            "model completed a {item_type} item more than once"
        ))),
        _ => Err(AgentError::Protocol(format!(
            "model completed an unknown {item_type} item"
        ))),
    }
}

pub(super) fn map_model_error(error: ModelError) -> AgentError {
    match error {
        ModelError::Cancelled => AgentError::Cancelled,
        ModelError::RequestRetriesExhausted {
            attempts, message, ..
        } => AgentError::ModelRequestRetriesExhausted { attempts, message },
        ModelError::StreamInterrupted(message) => AgentError::ModelStreamInterrupted(message),
        ModelError::UnsupportedImageInput {
            status,
            code,
            message,
            request_id,
        } => AgentError::UnsupportedImageInput {
            status,
            code,
            message,
            request_id,
        },
        ModelError::UnsupportedReasoning {
            status,
            code,
            message,
            request_id,
        } => AgentError::UnsupportedReasoning {
            status,
            code,
            message,
            request_id,
        },
        ModelError::ContextWindowExceeded {
            status,
            code,
            message,
            request_id,
        } => AgentError::ProviderContextWindowExceeded {
            status,
            code,
            message,
            request_id,
        },
        error => AgentError::Model(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
