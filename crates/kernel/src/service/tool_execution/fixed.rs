use super::*;

impl AgentService {
    /// Runs one call through the step's gate. The kernel's own tools and
    /// the plan's search hold it for the whole call; a plugin's handler
    /// takes it itself, since it lets go of it while it waits for an
    /// approval.
    pub(super) async fn execute_tool_call(
        &self,
        identity: TurnIdentity,
        call: &ModelToolCall,
        registration: &crate::AgentPlannedTool,
        runtime: &crate::ToolCallRuntime,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentToolOutput, AgentError> {
        let class = registration.execution_class;
        let gated = match registration.binding {
            crate::AgentPlannedToolBinding::PlanSearch => {
                runtime
                    .dispatch(class, async {
                        self.execute_plan_search(call, runtime.catalog())
                    })
                    .await
            }
            crate::AgentPlannedToolBinding::Handler => {
                return self
                    .execute_handler_tool_call(
                        identity,
                        call,
                        registration,
                        runtime,
                        cancellation,
                        event_sink,
                    )
                    .await;
            }
            crate::AgentPlannedToolBinding::Core(core_tool) => {
                runtime
                    .dispatch(
                        class,
                        self.execute_core_tool_call(
                            identity,
                            call,
                            core_tool,
                            cancellation,
                            event_sink,
                        ),
                    )
                    .await
            }
        };
        gated.map_err(|error| AgentError::Protocol(error.to_string()))?
    }

    /// Runs one of the kernel's own tools. They need no approval: each acts
    /// on the turn itself.
    async fn execute_core_tool_call(
        &self,
        identity: TurnIdentity,
        call: &ModelToolCall,
        core_tool: crate::AgentCoreTool,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentToolOutput, AgentError> {
        let typed_call = match CoreToolCall::parse(core_tool, &call.arguments) {
            Ok(typed_call) => typed_call,
            Err(error) => {
                return Ok(AgentToolOutput::model_visible_error(
                    json!({ "error": bounded_diagnostic(error.to_string()) }),
                ));
            }
        };
        let session_mode = self
            .session(identity.session_id)?
            .lock()?
            .settings
            .config
            .mode;
        if !core_tool.allowed_in_mode(session_mode) {
            return Ok(AgentToolOutput::model_visible_error(json!({
                "error": "tool is unavailable in the current Agent mode"
            })));
        }
        match typed_call {
            CoreToolCall::NewContext => self.request_new_context(identity).await,
            CoreToolCall::UpdatePlan(plan) => {
                self.update_task_plan(identity, plan, event_sink).await
            }
            CoreToolCall::Compression(tool) => {
                self.execute_compression_tool(identity, call, tool).await
            }
            CoreToolCall::RequestUserInput(request) => {
                self.request_user_input(
                    identity,
                    call.call_id.clone(),
                    request,
                    cancellation,
                    event_sink,
                )
                .await
            }
        }
    }

    /// Runs a compression tool on the session as it stands. A tool that
    /// changes the state only reports here: it is applied when its output is
    /// committed ([`Self::finish_tool_call`]), against the state of that
    /// moment, so the state and its receipt land in one commit and two calls
    /// of one step apply in order.
    async fn execute_compression_tool(
        &self,
        identity: TurnIdentity,
        call: &ModelToolCall,
        tool: crate::AgentCoreTool,
    ) -> Result<AgentToolOutput, AgentError> {
        let cell = self.session(identity.session_id)?;
        let guard = cell.lock_after_persistence().await?;
        exact_turn_identity(&guard, identity)?;
        let run = super::super::compression::compression_tool_output(
            &guard,
            tool,
            &call.call_id,
            &call.arguments,
            now_millis_u64(),
        );
        Ok(AgentToolOutput::typed(run.output))
    }
}
