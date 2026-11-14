//! What the facade asks of a parent's agents: their statuses, as an
//! interface lists them, and the interrupt of one.

use hanafubuki_agent_kernel::{AgentError, AgentSessionId, AgentSessionOrigin};

use super::{AgentRunStatus, SubagentService};
use crate::plugins::subagents::{AgentSubagentState, AgentSubagentStatus};

/// The longest last answer the facade's status carries.
const ANSWER_EXCERPT_CHARS: usize = 400;

impl SubagentService {
    /// The facade's view of `parent`'s agents, each with the model it runs
    /// on and its last answer cut to an excerpt.
    pub(crate) async fn statuses(
        &self,
        parent: AgentSessionId,
    ) -> Result<Vec<AgentSubagentStatus>, AgentError> {
        let mut statuses = Vec::new();
        for (agent, model) in self.agents_on_models(parent).await? {
            let last_answer = match &agent.status {
                AgentRunStatus::Completed(Some(answer)) => Some(answer.clone()),
                _ => {
                    let id = agent.id;
                    self.blocking(move |driver| driver.last_answer(id)).await?
                }
            };
            statuses.push(AgentSubagentStatus {
                session_id: agent.id,
                title: agent.name,
                status: match agent.status {
                    AgentRunStatus::Running => AgentSubagentState::Running,
                    AgentRunStatus::Errored(_) => AgentSubagentState::Failed,
                    _ => AgentSubagentState::Idle,
                },
                provider_id: model.provider_id,
                model: model.model,
                reasoning_effort: model.reasoning_effort,
                last_answer: last_answer.map(|answer| excerpt(&answer)),
            });
        }
        Ok(statuses)
    }

    /// Interrupts the turn the subagent `session_id` runs, if any.
    pub(crate) async fn interrupt_subagent(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        if self.session(session_id).await?.origin != Some(AgentSessionOrigin::Subagent) {
            return Err(AgentError::InvalidInput(format!(
                "Agent session {session_id} is not a subagent"
            )));
        }
        self.stop_turn(session_id).await
    }
}

fn excerpt(answer: &str) -> String {
    match answer.char_indices().nth(ANSWER_EXCERPT_CHARS) {
        Some((end, _)) => format!("{}…", &answer[..end]),
        None => answer.to_string(),
    }
}
