use super::*;

mod events;
mod finalize;
mod reasoning;

use events::stream_event;
pub(in crate::service) use reasoning::PendingReasoning;

/// What the model stream has produced so far, keyed by item in arrival order.
struct StreamedResponse {
    started_response_id: Option<String>,
    item_order: Vec<String>,
    pending_items: HashMap<String, PendingModelItem>,
    streamed_output_bytes: usize,
    saw_output_item: bool,
    plan_mode: bool,
    sampling_step: u32,
    /// The step's memory policy: what of a streaming answer is shown, and
    /// what the final answer cites.
    memory: crate::memory::StepMemoryPolicy,
    /// The provider's reason when the response stopped before it finished:
    /// the output streamed so far is kept, unfinished items as they stand.
    incomplete: Option<String>,
}

/// The completed response, ready to commit and publish.
struct FinalizedResponse {
    has_tool_calls: bool,
    items: Vec<AgentSamplingItem>,
    draft: Vec<ConversationItem>,
    tool_calls: Vec<ModelToolCall>,
    committed_memory_citations: Vec<crate::AgentMemoryCitationSnapshot>,
    completion_segments: Vec<(AgentMessageId, plan_output::PlanOutputSegment)>,
    completed_plans: Vec<AgentMessageId>,
    last_agent_message_id: Option<AgentMessageId>,
}

impl AgentService {
    #[allow(
        clippy::too_many_arguments,
        reason = "independent inputs that a parameter struct would only regroup"
    )]
    pub(super) async fn run_sampling_request_once(
        &self,
        identity: TurnIdentity,
        model: Arc<dyn ModelSession>,
        request: ModelRequest,
        tool_plan: AgentToolPlan,
        memory: crate::memory::StepMemoryPolicy,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<SamplingOutput, AgentError> {
        let sample_started = Instant::now();
        log::debug!(
            "agent_diag stage=model.stream.connect.begin session_id={} turn_id={} generation={}",
            identity.session_id,
            identity.turn_id,
            identity.generation
        );
        let estimated_input_tokens = estimate_model_request_tokens(&request)?;
        let mut stream = model
            .stream(request.clone(), cancellation.clone())
            .await
            .map_err(map_model_error)?;
        log::debug!(
            "agent_diag stage=model.stream.connect.end session_id={} turn_id={} generation={} outcome=ok elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            sample_started.elapsed().as_millis()
        );
        self.check_cancelled(identity)?;
        let sampling_step = self.current_sampling_step(identity)?;
        let plan_mode = self
            .session(identity.session_id)?
            .lock()?
            .settings
            .config
            .mode
            == crate::AgentSessionMode::Plan;
        let mut response = StreamedResponse {
            started_response_id: None,
            item_order: Vec::new(),
            pending_items: HashMap::new(),
            streamed_output_bytes: 0,
            saw_output_item: false,
            plan_mode,
            sampling_step,
            memory: memory.clone(),
            incomplete: None,
        };
        let (end_turn, completed_usage) = loop {
            let stream_idle_timeout = self
                .turn_budget_policy(identity)?
                .model_stream_idle_timeout();
            let stream_idle_timeout_seconds = stream_idle_timeout.as_secs() as usize;
            let event = tokio::time::timeout(stream_idle_timeout, stream.next())
                .await
                .map_err(|_| AgentError::LimitExceeded {
                    resource: "model stream idle timeout seconds",
                    limit: stream_idle_timeout_seconds,
                })?
                .ok_or_else(|| {
                    AgentError::Protocol(
                        "model stream ended before response completion".to_string(),
                    )
                })?;
            let event = stream_event(event, &response)?;
            self.check_cancelled(identity)?;
            if let Some(completion) = self
                .apply_model_event(identity, &mut response, event, event_sink)
                .await?
            {
                break completion;
            }
        };
        let streamed_output_bytes = response.streamed_output_bytes;
        let FinalizedResponse {
            has_tool_calls,
            items,
            draft,
            tool_calls,
            committed_memory_citations,
            completion_segments,
            completed_plans,
            last_agent_message_id,
        } = self.finalize_sampling_items(response, end_turn, &tool_plan)?;
        let estimated_output_tokens = estimate_conversation_items_tokens(&draft)?;
        self.commit_sampling_response(
            identity,
            &draft,
            completed_usage,
            estimated_input_tokens,
            estimated_output_tokens,
            event_sink,
        )
        .await?;
        for (message_id, segment) in completion_segments {
            self.emit_plan_output_segment(identity, message_id, sampling_step, segment, event_sink)
                .await?;
        }
        for message_id in completed_plans {
            self.emit(
                identity,
                AgentEventKind::ProposedPlanCompleted { message_id },
                event_sink,
            )
            .await?;
        }
        self.inner
            .registries
            .observers
            .notify(&AgentNotification::SamplingCommitted {
                session_id: identity.session_id,
                turn_id: identity.turn_id,
                generation: identity.generation,
                memory_citations: committed_memory_citations,
            });
        for item in &items {
            if let AgentSamplingItem::Assistant {
                message_id, phase, ..
            } = item
            {
                self.emit(
                    identity,
                    AgentEventKind::AssistantMessageCompleted {
                        message_id: *message_id,
                        phase: *phase,
                    },
                    event_sink,
                )
                .await?;
            }
        }
        let message_phase_present = items
            .iter()
            .any(|item| matches!(item, AgentSamplingItem::Assistant { phase: Some(_), .. }));
        let needs_follow_up = has_tool_calls || end_turn == Some(false);
        let provider_end_turn = match end_turn {
            Some(true) => "true",
            Some(false) => "false",
            None => "missing",
        };
        log::debug!(
            "agent_diag stage=model.stream.complete session_id={} turn_id={} generation={} sampling_step={} tool_calls={} message_phase_present={} provider_end_turn={} needs_follow_up={} output_bytes={} elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            sampling_step,
            tool_calls.len(),
            message_phase_present,
            provider_end_turn,
            needs_follow_up,
            streamed_output_bytes,
            sample_started.elapsed().as_millis()
        );
        if !tool_calls.is_empty() {
            self.run_tools(
                identity,
                tool_calls.clone(),
                tool_plan,
                &memory,
                cancellation,
                event_sink,
            )
            .await?;
        }
        Ok(SamplingOutput {
            result: SamplingRequestResult {
                needs_follow_up,
                last_agent_message_id,
            },
            items,
            tool_calls,
            provider_end_turn: end_turn,
        })
    }

    async fn emit_plan_output_segment(
        &self,
        identity: TurnIdentity,
        message_id: AgentMessageId,
        sampling_step: u32,
        segment: plan_output::PlanOutputSegment,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let kind = match segment {
            plan_output::PlanOutputSegment::Normal(delta) => {
                AgentEventKind::AssistantTextDelta { message_id, delta }
            }
            plan_output::PlanOutputSegment::PlanStart => AgentEventKind::ProposedPlanStarted {
                message_id,
                sampling_step,
            },
            plan_output::PlanOutputSegment::PlanDelta(delta) => {
                AgentEventKind::ProposedPlanDelta { message_id, delta }
            }
            plan_output::PlanOutputSegment::PlanEnd => return Ok(()),
        };
        self.emit(identity, kind, event_sink).await
    }
}
