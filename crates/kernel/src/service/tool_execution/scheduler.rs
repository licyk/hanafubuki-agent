use super::*;

/// A call that ran to an output, waiting for its commit.
struct CompletedCall {
    call: ModelToolCall,
    started: Instant,
    output: AgentToolOutput,
    memory_context_effect: crate::AgentMemoryContextEffect,
}

impl AgentService {
    /// Runs one step's calls together and commits each output as soon as
    /// every earlier call's has been, so history keeps call order. A call
    /// that ends in an error (the turn's cancellation among them) commits
    /// nothing; the others still run to their end and commit, because their
    /// effects happened, and the step then returns the first error. The
    /// calls left without an output get the synthetic "interrupted" one at
    /// the turn's terminal commit.
    pub(in crate::service) async fn run_tools(
        &self,
        identity: TurnIdentity,
        calls: Vec<ModelToolCall>,
        tool_plan: AgentToolPlan,
        memory: &crate::memory::StepMemoryPolicy,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.reserve_tool_invocations(identity, calls.len()).await?;
        let maximum_in_flight = self.turn_budget_policy(identity)?.maximum_in_flight_tools;
        let runtime = crate::ToolCallRuntime::new(tool_plan, maximum_in_flight)
            .map_err(|error| AgentError::Protocol(error.to_string()))?;
        let mut pending = Vec::with_capacity(calls.len());
        for call in calls {
            let disposition = CallDisposition::of(runtime.catalog(), &call);
            log::debug!(
                "agent_diag stage=tool.core.begin session_id={} turn_id={} generation={}",
                identity.session_id,
                identity.turn_id,
                identity.generation
            );
            self.check_cancelled(identity)?;
            trace_tool_call(identity, &call);
            self.set_phase_and_emit(
                identity,
                AgentTurnPhase::RunningTools,
                AgentEventKind::ToolStarted {
                    call_id: call.call_id.clone(),
                    name: disposition.tool_name(&call).to_string(),
                    presentation: disposition.presentation(),
                },
                event_sink,
            )
            .await?;
            pending.push(self.run_call(
                identity,
                call,
                disposition,
                &runtime,
                cancellation.clone(),
                event_sink,
            ));
        }
        let count = pending.len();
        let mut calls = crate::CompletionSet::new(pending);
        let mut results = std::iter::repeat_with(|| None)
            .take(count)
            .collect::<Vec<Option<Result<CompletedCall, AgentError>>>>();
        let mut next_commit = 0;
        let mut first_error = None;
        while let Some((index, result)) = calls.next().await {
            results[index] = Some(result);
            while let Some(result) = results.get_mut(next_commit).and_then(Option::take) {
                next_commit += 1;
                match result {
                    Ok(completed) => {
                        self.commit_completed_call(identity, completed, memory, event_sink)
                            .await?
                    }
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => self.check_cancelled(identity),
        }
    }

    /// One call, from its disposition to its output.
    async fn run_call(
        &self,
        identity: TurnIdentity,
        call: ModelToolCall,
        disposition: CallDisposition,
        runtime: &crate::ToolCallRuntime,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<CompletedCall, AgentError> {
        let started = Instant::now();
        let plan = runtime.catalog();
        let (output, rejected) = match disposition {
            CallDisposition::Run(ref registration) => (
                self.execute_tool_call(
                    identity,
                    &normalized_call(&call, registration),
                    registration,
                    runtime,
                    cancellation,
                    event_sink,
                )
                .await?,
                false,
            ),
            CallDisposition::Reject { ref output, .. } => {
                log::debug!(
                    "agent_diag stage=tool.reject session_id={} turn_id={} generation={} outcome=recoverable",
                    identity.session_id,
                    identity.turn_id,
                    identity.generation
                );
                (AgentToolOutput::clone(output), true)
            }
        };
        let memory_context_effect = match &disposition {
            _ if rejected || output.is_error() => crate::AgentMemoryContextEffect::None,
            CallDisposition::Run(registration)
                if matches!(
                    registration.binding,
                    crate::AgentPlannedToolBinding::PlanSearch
                ) && plan.selected_tools_pollute_memory(&output.selected_tools) =>
            {
                crate::AgentMemoryContextEffect::PollutesContribution
            }
            CallDisposition::Run(registration) => registration.memory_context_effect,
            CallDisposition::Reject { .. } => crate::AgentMemoryContextEffect::None,
        };
        self.emit(
            identity,
            AgentEventKind::ToolExecutionFinished {
                call_id: call.call_id.clone(),
                is_error: output.is_error(),
            },
            event_sink,
        )
        .await?;
        Ok(CompletedCall {
            call,
            started,
            output,
            memory_context_effect,
        })
    }

    async fn commit_completed_call(
        &self,
        identity: TurnIdentity,
        completed: CompletedCall,
        memory: &crate::memory::StepMemoryPolicy,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let CompletedCall {
            call,
            started,
            output,
            memory_context_effect,
        } = completed;
        self.finish_tool_call(identity, &call, output, event_sink)
            .await?;
        if memory_context_effect == crate::AgentMemoryContextEffect::PollutesContribution {
            let pollution = match memory.excludes_external_context() {
                Ok(true) => self
                    .mark_session_memory_contribution_polluted(identity.session_id)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                Ok(false) => Ok(()),
                Err(error) => Err(error),
            };
            if let Err(error) = pollution {
                log::warn!(
                    "Agent memory contribution pollution could not be recorded after tool output commit: {}",
                    bounded_diagnostic(error)
                );
            }
        }
        log::debug!(
            "agent_diag stage=tool.core.end session_id={} turn_id={} generation={} outcome=committed elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            started.elapsed().as_millis()
        );
        Ok(())
    }
}

/// The call as the tool receives it: its arguments through the kernel's
/// schema-guided pass (`1.0` and numeric strings to numbers, a null optional
/// to absent). History keeps the model's own arguments.
fn normalized_call(call: &ModelToolCall, registration: &crate::AgentPlannedTool) -> ModelToolCall {
    let mut call = call.clone();
    if call.arguments.is_object() {
        call.arguments = crate::tool_catalog::normalize_tool_arguments(
            &registration.model_spec.input_schema,
            &call.arguments,
        );
    }
    call
}
