//! What the facade asks of a parent's agents: their statuses, as an
//! interface lists them (and `background_tasks` lists a tree's), and the
//! interrupt of one.

use hanafubuki_agent_kernel::{AgentError, AgentSessionId, AgentSessionOrigin};

use super::{AgentRunStatus, SubagentRuns, SubagentService};
use crate::plugins::subagents::{AgentSubagentState, AgentSubagentStatus};
use crate::session_driver::AgentSessionDriver;

/// The longest last answer the facade's status carries.
const ANSWER_EXCERPT_CHARS: usize = 400;

impl SubagentService {
    /// The facade's view of `parent`'s agents, each with the model it runs
    /// on and its last answer cut to an excerpt.
    pub(crate) async fn statuses(
        &self,
        parent: AgentSessionId,
    ) -> Result<Vec<AgentSubagentStatus>, AgentError> {
        let runs = self.runs.clone();
        self.blocking(move |driver| runs.statuses(driver, parent, true))
            .await
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

impl SubagentRuns {
    /// `parent`'s agents as an interface lists them, their runs read at
    /// once; with their last answers when `answers` says so (each may read
    /// the agent's conversation), else none. Blocks.
    pub(crate) fn statuses(
        &self,
        driver: &AgentSessionDriver,
        parent: AgentSessionId,
        answers: bool,
    ) -> Result<Vec<AgentSubagentStatus>, AgentError> {
        let mut statuses = Vec::new();
        for (agent, model) in self.agents_on_models(driver, parent)? {
            let last_answer = match (&agent.status, answers) {
                (_, false) => None,
                (AgentRunStatus::Completed(Some(answer)), true) => Some(answer.clone()),
                (_, true) => driver.last_answer(agent.id)?,
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
                state_since: agent.since,
            });
        }
        Ok(statuses)
    }
}

fn excerpt(answer: &str) -> String {
    match answer.char_indices().nth(ANSWER_EXCERPT_CHARS) {
        Some((end, _)) => format!("{}…", &answer[..end]),
        None => answer.to_string(),
    }
}
