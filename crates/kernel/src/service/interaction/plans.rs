use super::*;

impl AgentService {
    pub(in crate::service) async fn update_task_plan(
        &self,
        identity: TurnIdentity,
        call: UpdateTaskPlanTool,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentToolOutput, AgentError> {
        let (staged, event) = {
            let cell = self.session(identity.session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let turn = exact_turn(record, identity.turn_id, identity.generation)?;
            if turn.phase.is_terminal() {
                return Err(AgentError::TurnNotRunning);
            }
            if record.settings.config.mode != crate::AgentSessionMode::Default {
                return Ok(AgentToolOutput::model_visible_error(json!({
                    "error": "update_plan is available only in Default mode"
                })));
            }
            let previous = record.rollback_state();
            let plan = AgentTaskPlanSnapshot {
                turn_id: identity.turn_id,
                generation: identity.generation,
                revision: record
                    .runtime
                    .task_plan
                    .as_ref()
                    .filter(|plan| {
                        plan.turn_id == identity.turn_id && plan.generation == identity.generation
                    })
                    .map_or(1, |plan| plan.revision.saturating_add(1)),
                explanation: call.explanation,
                plan: call.plan,
                updated_at: now_millis(),
            };
            record.runtime.task_plan = Some(plan.clone());
            let event = record_event(
                record,
                identity,
                AgentEventKind::TaskPlanUpdated { plan },
                self.inner.limits,
            );
            (StagedPersistence::commit(&cell, record, previous), event)
        };
        self.persist_staged(staged).await?;
        event_sink.emit(event);
        Ok(AgentToolOutput::success(
            json!({ "message": "Plan updated" }),
        ))
    }
}
