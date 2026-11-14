//! What `exec` does with a sub-agent's approvals and questions, which the
//! Agent tells it as they are asked (`AgentBackgroundEvent::Child`),
//! during the session's turn or while it waits: an approval is answered by
//! `--approve`, as the session's are, and noted with the sub-agent's title;
//! a question is answered by the person at a terminal, and without one the
//! sub-agent and its live subtree are closed (the run goes on, and its exit code is
//! still the session's last turn's).

use hanafubuki_agent::{AgentEvent, AgentEventKind, AgentSessionId};

use super::Turn;
use crate::agent::block_on;

impl Turn<'_> {
    pub(super) fn close_background(&mut self, root: AgentSessionId) {
        let closed = block_on(self.agent, move |agent| async move {
            let tree = agent.background_tasks(root).await?;
            for child in tree
                .agents
                .into_iter()
                .filter(|child| child.parent_session_id == root)
            {
                let live = matches!(
                    child.status,
                    hanafubuki_agent::AgentSubagentState::Running
                        | hanafubuki_agent::AgentSubagentState::Waiting
                ) || !agent.background_status(child.session_id)?.work.is_empty();
                if live {
                    agent.close_subagent(child.session_id).await?;
                }
            }
            Ok::<(), hanafubuki_agent::AgentError>(())
        });
        if let Err(failure) = closed {
            self.output.note(&format!(
                "background agents were not closed: {}",
                failure.message
            ));
        }
        match self.agent.stop_background_terminals_confirmed(root) {
            Ok(outcome) if outcome.retained.is_empty() => {}
            Ok(_) => self
                .output
                .note("some background terminals could not be stopped"),
            Err(error) => self
                .output
                .note(&format!("background terminals were not closed: {error}")),
        }
    }

    /// Answers what `event`, an event of a sub-agent's turn, asks.
    pub(super) fn child(&mut self, event: AgentEvent) {
        let child = event.session_id;
        let Some(turn_id) = event.turn_id else {
            return;
        };
        match &event.kind {
            AgentEventKind::ApprovalRequested { request } => {
                let tool = self.child_tool(child, &request.operation_id);
                let title = self.title(child);
                let decision =
                    (self.answers).approval(request, tool.as_deref(), Some(&title), self.output);
                if let Err(error) = self.agent.resolve_approval(
                    child,
                    turn_id,
                    event.generation,
                    request.request_id,
                    decision,
                ) {
                    self.output.note(&format!(
                        "the approval of sub-agent {title} was not answered: {error}"
                    ));
                }
            }
            AgentEventKind::UserInputRequested { request } => {
                let title = self.title(child);
                match self.answers.questions(request, Some(&title), self.output) {
                    Some(answers) => {
                        let answered = self.agent.resolve_user_input(
                            child,
                            turn_id,
                            event.generation,
                            request.request_id,
                            answers,
                        );
                        if let Err(error) = answered {
                            self.output.note(&format!(
                                "the question of sub-agent {title} was not answered: {error}"
                            ));
                        }
                    }
                    None => self.close_child(child, &title),
                }
            }
            _ => {}
        }
    }

    /// Closes `child`, whose question no one can answer.
    fn close_child(&mut self, child: AgentSessionId, title: &str) {
        self.output.note(&format!(
            "sub-agent {title} asked a question that no one at a terminal can answer; \
             closing it"
        ));
        let closed = block_on(self.agent, move |agent| async move {
            agent.close_subagent(child).await
        });
        if let Err(failure) = closed {
            self.output.note(&format!(
                "sub-agent {title} was not closed: {}",
                failure.message
            ));
        }
    }

    /// The tool of `child`'s call `call_id`, from its journal: what a
    /// sub-agent's turn tells names no tool.
    fn child_tool(&self, child: AgentSessionId, call_id: &str) -> Option<String> {
        let journal = self.agent.events_since(child, None).ok()?;
        journal
            .events
            .into_iter()
            .find_map(|event| match event.kind {
                AgentEventKind::ToolStarted {
                    call_id: started,
                    name,
                    ..
                } if started == call_id => Some(name),
                _ => None,
            })
    }

    /// `child`'s title, its task name.
    fn title(&self, child: AgentSessionId) -> String {
        self.agent
            .snapshot(child)
            .ok()
            .and_then(|snapshot| snapshot.title)
            .map_or_else(|| child.to_string(), |title| title.value)
    }
}
