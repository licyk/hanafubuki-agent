//! Turning the completed stream's items into the draft conversation items,
//! sampling items, and tool calls the turn commits.

use super::*;

impl AgentService {
    pub(super) fn finalize_sampling_items(
        &self,
        mut response: StreamedResponse,
        end_turn: Option<bool>,
        tool_plan: &AgentToolPlan,
    ) -> Result<FinalizedResponse, AgentError> {
        let sampling_step = response.sampling_step;
        let has_tool_calls = response
            .pending_items
            .values()
            .any(|item| matches!(item, PendingModelItem::ToolCall(_)));
        let mut items = Vec::with_capacity(response.item_order.len());
        let mut draft = Vec::with_capacity(response.item_order.len());
        let mut tool_calls = Vec::new();
        let mut committed_memory_citations = Vec::new();
        let mut completion_segments = Vec::new();
        let mut completed_plans = Vec::new();
        let mut last_agent_message_id = None;
        for item_id in response.item_order {
            let item = response.pending_items.remove(&item_id).ok_or_else(|| {
                AgentError::Protocol("model output item ordering became inconsistent".to_string())
            })?;
            match item {
                PendingModelItem::Assistant {
                    message_id,
                    provider_phase,
                    streamed_text,
                    filtered_text_bytes,
                    mut plan_output,
                    completed,
                } => {
                    // A response that stopped early keeps the text streamed
                    // so far of an item it did not finish.
                    let (completed_phase, text) = match completed {
                        Some(completed) => completed,
                        None if response.incomplete.is_some() => (None, streamed_text.clone()),
                        None => {
                            return Err(AgentError::Protocol(
                                "model response completed with an unfinished assistant item"
                                    .to_string(),
                            ));
                        }
                    };
                    let provider_phase = completed_phase.or(provider_phase);
                    let phase = provider_phase.map(AgentMessagePhase::from);
                    if phase != Some(AgentMessagePhase::Commentary) {
                        last_agent_message_id = Some(message_id);
                    }
                    let processed = if !has_tool_calls
                        && end_turn != Some(false)
                        && phase != Some(AgentMessagePhase::Commentary)
                    {
                        response
                            .memory
                            .process_final_answer(&text)
                            .map_err(AgentError::Protocol)?
                    } else {
                        hanafubuki_agent_contracts::AgentProcessedMemoryAnswer {
                            visible_text: text,
                            citations: Vec::new(),
                        }
                    };
                    let filtered_text =
                        streamed_text.get(..filtered_text_bytes).ok_or_else(|| {
                            AgentError::Protocol(
                                "memory stream filter split an assistant character".to_string(),
                            )
                        })?;
                    if !processed.visible_text.starts_with(filtered_text) {
                        return Err(AgentError::Protocol(
                            "completed assistant text changed an already presented prefix"
                                .to_string(),
                        ));
                    }
                    let tail = &processed.visible_text[filtered_text_bytes..];
                    let (visible_text, proposed_plan) = if let Some(output) = plan_output.as_mut() {
                        let mut segments = output.push_str(tail);
                        segments.extend(output.finish());
                        completion_segments
                            .extend(segments.into_iter().map(|segment| (message_id, segment)));
                        let proposed_plan = output.proposed_plan().map(|text| AgentPlanBlock {
                            text: text.to_string(),
                        });
                        (output.visible_text().to_string(), proposed_plan)
                    } else {
                        if !tail.is_empty() {
                            completion_segments.push((
                                message_id,
                                plan_output::PlanOutputSegment::Normal(tail.to_string()),
                            ));
                        }
                        (processed.visible_text.clone(), None)
                    };
                    let model_text = (visible_text != processed.visible_text)
                        .then(|| processed.visible_text.clone());
                    if proposed_plan.is_some() {
                        completed_plans.push(message_id);
                    }
                    committed_memory_citations.extend(processed.citations.iter().cloned());
                    draft.push(ConversationItem::AssistantMessage {
                        message_id,
                        phase,
                        text: visible_text.clone(),
                        model_text,
                        proposed_plan,
                        sampling_step,
                        memory_citations: processed.citations,
                    });
                    items.push(AgentSamplingItem::Assistant {
                        message_id,
                        phase,
                        text: visible_text,
                    });
                }
                PendingModelItem::ToolCall(call) => {
                    if tool_calls.len() >= self.inner.limits.maximum_concurrent_tool_calls {
                        return Err(AgentError::LimitExceeded {
                            resource: "tool calls per sampling step",
                            limit: self.inner.limits.maximum_concurrent_tool_calls,
                        });
                    }
                    // A name the step does not offer is recorded under the
                    // unavailable identity; the scheduler answers it with
                    // the same error as any absent tool.
                    let (provider_id, tool_id, tool_generation) = match tool_plan
                        .resolve_call(&call)
                        .ok()
                        .or_else(|| tool_plan.unselected_deferred_call(&call))
                    {
                        Some(registration) => (
                            registration.provider_id.clone(),
                            registration.tool_id.clone(),
                            registration.generation,
                        ),
                        None => crate::service::tool_execution::unavailable_tool_identity(),
                    };
                    draft.push(ConversationItem::ToolCall {
                        call_id: call.call_id.clone(),
                        namespace: call.namespace.clone(),
                        name: call.name.clone(),
                        origin: call.origin(),
                        catalog_revision: tool_plan.revision,
                        provider_id,
                        tool_id,
                        tool_generation,
                        arguments: call.arguments.clone(),
                    });
                    tool_calls.push(call.clone());
                    items.push(AgentSamplingItem::ToolCall(call));
                }
                PendingModelItem::Reasoning { completed, .. } => {
                    if !completed && response.incomplete.is_none() {
                        return Err(AgentError::Protocol(
                            "model response completed with an unfinished reasoning item"
                                .to_string(),
                        ));
                    }
                }
                PendingModelItem::Ignored {
                    item_type,
                    completed,
                } => {
                    if !completed && response.incomplete.is_none() {
                        return Err(AgentError::Protocol(format!(
                            "model response completed with an unfinished {item_type} item"
                        )));
                    }
                }
            }
        }
        if draft.is_empty() {
            return Err(AgentError::Protocol(
                "model response committed no conversation item".to_string(),
            ));
        }
        Ok(FinalizedResponse {
            has_tool_calls,
            items,
            draft,
            tool_calls,
            committed_memory_citations,
            completion_segments,
            completed_plans,
            last_agent_message_id,
        })
    }
}
