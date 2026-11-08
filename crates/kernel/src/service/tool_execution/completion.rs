use super::*;

impl AgentService {
    pub(super) async fn finish_tool_call(
        &self,
        identity: TurnIdentity,
        call: &ModelToolCall,
        output: AgentToolOutput,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let (output, truncation) = crate::tool_output::canonicalize_agent_tool_output(
            output,
            self.inner.limits.maximum_tool_output_bytes,
        );
        trace_tool_result(identity, &output);
        let is_error = output.is_error();
        let skill_context = output.skill_context;
        let images = output
            .prepared_images
            .into_iter()
            .map(|image| {
                self.inner
                    .history_store
                    .stage_image_attachment(identity.session_id, image)
                    .map_err(history_store_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut committed_items = vec![ConversationItem::ToolOutput {
            call_id: call.call_id.clone(),
            origin: call.origin(),
            output: output.output,
            truncation,
            images,
            tool_search_results: output.tool_search_results,
        }];
        if !output.selected_tools.is_empty() {
            committed_items.push(ConversationItem::ToolSelection {
                call_id: call.call_id.clone(),
                tools: output.selected_tools,
            });
        }
        if let Some(skill_context) = skill_context {
            if !self.skill_is_active_in_turn(identity, &skill_context.skill)? {
                committed_items.push(ConversationItem::SkillContext {
                    skill: skill_context.skill,
                    name: skill_context.name,
                    content: skill_context.content,
                });
            }
        }
        self.commit_history(identity, &committed_items, event_sink)
            .await?;
        self.emit(
            identity,
            AgentEventKind::ToolCompleted {
                call_id: call.call_id.clone(),
                is_error,
            },
            event_sink,
        )
        .await
    }
}
