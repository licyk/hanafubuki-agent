//! Applying each model stream event to the response being assembled.

use super::*;

/// Map a model stream error by how far the response had progressed.
pub(super) fn stream_event(
    event: Result<ModelEvent, ModelError>,
    response: &StreamedResponse,
) -> Result<ModelEvent, AgentError> {
    match event {
        Ok(event) => Ok(event),
        Err(ModelError::StreamInterrupted(message)) if response.started_response_id.is_none() => {
            Err(AgentError::ModelStreamInterrupted(message))
        }
        Err(ModelError::IdleTimeout { timeout }) if response.started_response_id.is_none() => {
            Err(AgentError::ModelStreamInterrupted(format!(
                "model response stream was idle for {timeout:?}"
            )))
        }
        Err(ModelError::StreamInterrupted(message)) => Err(AgentError::Model(format!(
            "model response stream interrupted after the first model event: {message}"
        ))),
        Err(error @ ModelError::UnsupportedImageInput { .. })
            if response.started_response_id.is_none() =>
        {
            Err(map_model_error(error))
        }
        Err(error @ ModelError::UnsupportedImageInput { .. }) => {
            Err(AgentError::Model(error.to_string()))
        }
        Err(error @ ModelError::ContextWindowExceeded { .. }) if !response.saw_output_item => {
            Err(map_model_error(error))
        }
        Err(error @ ModelError::ContextWindowExceeded { .. }) => {
            Err(AgentError::Model(error.to_string()))
        }
        Err(error) => Err(map_model_error(error)),
    }
}

impl AgentService {
    /// Apply one model event. Returns the provider's end-turn flag and usage
    /// once the response completes.
    pub(super) async fn apply_model_event(
        &self,
        identity: TurnIdentity,
        response: &mut StreamedResponse,
        event: ModelEvent,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<(Option<bool>, Option<hanafubuki_agent_model_api::ModelUsage>)>, AgentError>
    {
        match event {
            ModelEvent::ResponseStarted { response_id, .. } => {
                if response
                    .started_response_id
                    .replace(response_id.clone())
                    .is_some()
                {
                    return Err(AgentError::Protocol(
                        "model stream started more than one response".to_string(),
                    ));
                }
                validate_bounded_identifier(&response_id)?;
                self.emit(
                    identity,
                    AgentEventKind::ResponseStarted { response_id },
                    event_sink,
                )
                .await?;
            }
            ModelEvent::OutputItemAdded { item } => {
                self.output_item_added(identity, response, item, event_sink)
                    .await?;
            }
            ModelEvent::ReasoningDelta {
                item_id,
                kind,
                index,
                delta,
            } => {
                require_response_started(&response.started_response_id)?;
                // Thinking is not output for the model output bound; the
                // reasoning text bound caps each item instead.
                let limit = self.inner.limits.maximum_reasoning_text_bytes;
                let reasoning = match response.pending_items.get_mut(&item_id) {
                    Some(PendingModelItem::Reasoning(reasoning)) if !reasoning.is_completed() => {
                        reasoning
                    }
                    _ => {
                        return Err(AgentError::Protocol(
                            "model streamed reasoning text for an unknown or completed reasoning item"
                                .to_string(),
                        ));
                    }
                };
                if let Some(delta) = reasoning.push(kind, index, &delta, limit) {
                    let reasoning_id = reasoning.reasoning_id;
                    self.emit(
                        identity,
                        AgentEventKind::ReasoningDelta {
                            reasoning_id,
                            delta,
                        },
                        event_sink,
                    )
                    .await?;
                }
            }
            ModelEvent::AssistantTextDelta { item_id, delta } => {
                self.assistant_text_delta(identity, response, item_id, delta, event_sink)
                    .await?;
            }
            ModelEvent::OutputItemCompleted { item } => {
                self.output_item_completed(identity, response, item, event_sink)
                    .await?;
            }
            ModelEvent::ResponseCompleted {
                response_id,
                usage,
                end_turn,
            } => {
                let started_id = require_response_started(&response.started_response_id)?;
                if started_id != &response_id {
                    return Err(AgentError::Protocol(
                        "response completion id does not match response start".to_string(),
                    ));
                }
                return Ok(Some((end_turn, usage)));
            }
            ModelEvent::ResponseIncomplete {
                response_id,
                usage,
                reason,
            } => {
                let started_id = require_response_started(&response.started_response_id)?;
                if started_id != &response_id {
                    return Err(AgentError::Protocol(
                        "response completion id does not match response start".to_string(),
                    ));
                }
                log::warn!(
                    "Agent model response stopped early ({}); its partial output is kept",
                    bounded_diagnostic(reason.clone())
                );
                response.incomplete = Some(reason);
                self.complete_unfinished_reasoning(identity, response, event_sink)
                    .await?;
                return Ok(Some((None, usage)));
            }
        }
        Ok(None)
    }

    async fn output_item_added(
        &self,
        identity: TurnIdentity,
        response: &mut StreamedResponse,
        item: ModelOutputItem,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let sampling_step = response.sampling_step;
        let plan_mode = response.plan_mode;
        response.saw_output_item = true;
        require_response_started(&response.started_response_id)?;
        let (item_id, pending) = match item {
            ModelOutputItem::AssistantMessage { item_id, phase, .. } => {
                let message_id = self.reserve_message_id(identity).await?;
                self.emit(
                    identity,
                    AgentEventKind::AssistantMessageStarted {
                        message_id,
                        phase: phase.map(Into::into),
                        sampling_step,
                    },
                    event_sink,
                )
                .await?;
                (
                    item_id,
                    PendingModelItem::Assistant {
                        message_id,
                        provider_phase: phase,
                        streamed_text: String::new(),
                        filtered_text_bytes: 0,
                        plan_output: plan_mode.then(plan_output::PlanOutputProjection::default),
                        completed: None,
                    },
                )
            }
            // Tool previews carry no presentation or durable state.
            // The completed item is the only execution authority.
            ModelOutputItem::ToolCall { .. } => return Ok(()),
            ModelOutputItem::Reasoning { item_id, .. } => {
                let reasoning_id = self.reserve_reasoning_id(identity).await?;
                (
                    item_id,
                    PendingModelItem::Reasoning(PendingReasoning::new(reasoning_id)),
                )
            }
            ModelOutputItem::Compaction { .. } => {
                return Err(AgentError::Protocol(
                    "ordinary sampling returned a compaction output item".to_string(),
                ));
            }
            ModelOutputItem::Other { item_id, item_type } => (
                item_id.unwrap_or_else(|| {
                    format!("anonymous-output-item-{}", response.item_order.len())
                }),
                PendingModelItem::Ignored {
                    item_type,
                    completed: false,
                },
            ),
        };
        validate_bounded_identifier(&item_id)?;
        if response.pending_items.contains_key(&item_id) {
            return Err(AgentError::Protocol(
                "model output item started more than once".to_string(),
            ));
        }
        response.item_order.push(item_id.clone());
        let reasoning_id = match &pending {
            PendingModelItem::Reasoning(reasoning) => Some(reasoning.reasoning_id),
            _ => None,
        };
        response.pending_items.insert(item_id, pending);
        if let Some(reasoning_id) = reasoning_id {
            self.reasoning_item_started(identity, event_sink).await?;
            self.emit(
                identity,
                AgentEventKind::ReasoningStarted {
                    reasoning_id,
                    sampling_step,
                },
                event_sink,
            )
            .await?;
        }
        Ok(())
    }

    async fn assistant_text_delta(
        &self,
        identity: TurnIdentity,
        response: &mut StreamedResponse,
        item_id: String,
        delta: String,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let sampling_step = response.sampling_step;
        require_response_started(&response.started_response_id)?;
        response.streamed_output_bytes = response.streamed_output_bytes.saturating_add(delta.len());
        enforce_size(
            "model output bytes",
            response.streamed_output_bytes,
            self.inner.limits.maximum_model_output_bytes,
        )?;
        let (message_id, presentation_segments) = match response.pending_items.get_mut(&item_id) {
            Some(PendingModelItem::Assistant {
                message_id,
                streamed_text,
                filtered_text_bytes,
                plan_output,
                completed: None,
                ..
            }) => {
                streamed_text.push_str(&delta);
                let visible_prefix_len = response
                    .memory
                    .visible_stream_prefix_len(streamed_text)
                    .map_err(AgentError::Protocol)?;
                if visible_prefix_len < *filtered_text_bytes
                    || visible_prefix_len > streamed_text.len()
                    || !streamed_text.is_char_boundary(visible_prefix_len)
                {
                    return Err(AgentError::Protocol(
                        "memory stream filter returned an invalid prefix".to_string(),
                    ));
                }
                let segments = if visible_prefix_len > *filtered_text_bytes {
                    let visible_delta = &streamed_text[*filtered_text_bytes..visible_prefix_len];
                    *filtered_text_bytes = visible_prefix_len;
                    if let Some(plan_output) = plan_output {
                        plan_output.push_str(visible_delta)
                    } else {
                        vec![plan_output::PlanOutputSegment::Normal(
                            visible_delta.to_string(),
                        )]
                    }
                } else {
                    Vec::new()
                };
                (*message_id, segments)
            }
            Some(PendingModelItem::Assistant {
                completed: Some(_), ..
            }) => {
                return Err(AgentError::Protocol(
                    "model emitted a delta after completing its assistant item".to_string(),
                ));
            }
            _ => {
                return Err(AgentError::Protocol(
                    "model assistant delta references an unknown item".to_string(),
                ));
            }
        };
        for segment in presentation_segments {
            self.emit_plan_output_segment(identity, message_id, sampling_step, segment, event_sink)
                .await?;
        }
        Ok(())
    }

    async fn output_item_completed(
        &self,
        identity: TurnIdentity,
        response: &mut StreamedResponse,
        item: ModelOutputItem,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        require_response_started(&response.started_response_id)?;
        // A provider that completes an item it never announced has still
        // sent the whole item: start it here, then complete it as usual.
        if let Some(start) = unannounced_item_start(response, &item) {
            log::debug!("agent_diag stage=model.stream.item_done_without_added");
            self.output_item_added(identity, response, start, event_sink)
                .await?;
        }
        match item {
            ModelOutputItem::AssistantMessage {
                item_id,
                phase,
                text,
            } => {
                enforce_size(
                    "model output bytes",
                    text.len(),
                    self.inner.limits.maximum_model_output_bytes,
                )?;
                match response.pending_items.get_mut(&item_id) {
                    Some(PendingModelItem::Assistant {
                        provider_phase,
                        completed,
                        ..
                    }) if completed.is_none() => {
                        // The completed item is authoritative for
                        // its phase, and finalization already
                        // prefers it. A model that opens an item as
                        // commentary and completes it as the final
                        // answer has refined its own annotation,
                        // not violated the protocol; rejecting that
                        // threw away the whole turn's work over a
                        // label the completion had already
                        // corrected. The shell is told the final
                        // phase by `AssistantMessageCompleted`.
                        if matches!(
                            (*provider_phase, phase),
                            (Some(started), Some(finished)) if started != finished
                        ) {
                            log::debug!(
                                "agent_diag stage=model.stream.phase_refined item_id={item_id}"
                            );
                        }
                        *completed = Some((phase, text));
                    }
                    Some(PendingModelItem::Assistant { .. }) => {
                        return Err(AgentError::Protocol(
                            "model completed an assistant item more than once".to_string(),
                        ));
                    }
                    _ => {
                        return Err(AgentError::Protocol(
                            "model completed an unknown assistant item".to_string(),
                        ));
                    }
                }
            }
            ModelOutputItem::ToolCall { mut call } => {
                validate_bounded_identifier(&call.item_id)?;
                validate_bounded_identifier(&call.call_id)?;
                validate_bounded_identifier(&call.name)?;
                if let Some(namespace) = &call.namespace {
                    validate_bounded_identifier(namespace)?;
                }
                bound_tool_arguments(&mut call, self.inner.limits.maximum_tool_argument_bytes)?;
                match response.pending_items.get_mut(&call.item_id) {
                    Some(PendingModelItem::ToolCall(_)) => {
                        return Err(AgentError::Protocol(
                            "model completed a tool item more than once".to_string(),
                        ));
                    }
                    Some(pending @ PendingModelItem::Ignored { .. }) => {
                        *pending = PendingModelItem::ToolCall(call);
                    }
                    Some(_) => {
                        return Err(AgentError::Protocol(
                            "model completed an item as two incompatible types".to_string(),
                        ));
                    }
                    None => {
                        response.item_order.push(call.item_id.clone());
                        response
                            .pending_items
                            .insert(call.item_id.clone(), PendingModelItem::ToolCall(call));
                    }
                }
            }
            ModelOutputItem::Reasoning {
                item_id,
                summary,
                content,
            } => {
                let limit = self.inner.limits.maximum_reasoning_text_bytes;
                let (reasoning_id, tail, duration_millis) = match response
                    .pending_items
                    .get_mut(&item_id)
                {
                    Some(PendingModelItem::Reasoning(reasoning)) if !reasoning.is_completed() => {
                        let (tail, duration_millis) = reasoning.complete(summary, content, limit);
                        (reasoning.reasoning_id, tail, duration_millis)
                    }
                    Some(PendingModelItem::Reasoning(_)) => {
                        return Err(AgentError::Protocol(
                            "model completed a reasoning item more than once".to_string(),
                        ));
                    }
                    _ => {
                        return Err(AgentError::Protocol(
                            "model completed an unknown reasoning item".to_string(),
                        ));
                    }
                };
                self.reasoning_completed(identity, reasoning_id, tail, duration_millis, event_sink)
                    .await?;
            }
            ModelOutputItem::Compaction { .. } => {
                return Err(AgentError::Protocol(
                    "ordinary sampling completed a compaction output item".to_string(),
                ));
            }
            ModelOutputItem::Other { item_id, item_type } => {
                complete_ignored_model_item(
                    &response.item_order,
                    &mut response.pending_items,
                    item_id.as_deref(),
                    &item_type,
                )?;
            }
        }
        Ok(())
    }
}

/// Arguments over the argument bound are not kept: the call records empty
/// arguments and carries the error, which the scheduler answers with a
/// model-visible error, so the turn goes on and history stays bounded.
fn bound_tool_arguments(
    call: &mut hanafubuki_agent_model_api::ModelToolCall,
    maximum_bytes: usize,
) -> Result<(), AgentError> {
    let raw = match &call.arguments {
        serde_json::Value::String(text) => text.clone(),
        arguments => serde_json::to_string(arguments)
            .map_err(|error| AgentError::Protocol(error.to_string()))?,
    };
    if raw.len() <= maximum_bytes {
        return Ok(());
    }
    log::debug!(
        "agent_diag stage=model.stream.tool_arguments_oversized bytes={} limit={maximum_bytes}",
        raw.len()
    );
    call.argument_error = Some(hanafubuki_agent_model_api::ModelToolArgumentError::new(
        format!(
            "the arguments are {} bytes, over the {maximum_bytes}-byte limit",
            raw.len()
        ),
        &raw,
    ));
    // A freeform call keeps a string so its origin does not change.
    call.arguments = if call.arguments.is_string() {
        serde_json::Value::String(String::new())
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };
    Ok(())
}

/// The start of a completed item no `OutputItemAdded` announced, or `None`
/// when it was announced (or needs no start: a tool call, whose completed
/// item is its only authority).
fn unannounced_item_start(
    response: &StreamedResponse,
    item: &ModelOutputItem,
) -> Option<ModelOutputItem> {
    match item {
        ModelOutputItem::AssistantMessage { item_id, phase, .. } => (!response
            .pending_items
            .contains_key(item_id))
        .then(|| ModelOutputItem::AssistantMessage {
            item_id: item_id.clone(),
            phase: *phase,
            text: String::new(),
        }),
        ModelOutputItem::Reasoning { item_id, .. } => {
            (!response.pending_items.contains_key(item_id)).then(|| ModelOutputItem::Reasoning {
                item_id: item_id.clone(),
                summary: Vec::new(),
                content: Vec::new(),
            })
        }
        ModelOutputItem::Other {
            item_id: Some(item_id),
            ..
        } => (!response.pending_items.contains_key(item_id)).then(|| item.clone()),
        ModelOutputItem::Other {
            item_id: None,
            item_type,
        } => (!response.pending_items.values().any(|pending| {
            matches!(
                pending,
                PendingModelItem::Ignored {
                    item_type: pending_type,
                    completed: false,
                } if pending_type == item_type
            )
        }))
        .then(|| item.clone()),
        ModelOutputItem::ToolCall { .. } | ModelOutputItem::Compaction { .. } => None,
    }
}
