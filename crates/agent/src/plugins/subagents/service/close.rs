//! A person's close stops the selected subtree and reports only its root.

use super::*;

impl SubagentService {
    pub(crate) async fn close_subagent(&self, child: AgentSessionId) -> Result<(), AgentError> {
        let session = self.session(child).await?;
        let parent = session
            .parent_session_id
            .filter(|_| session.origin == Some(AgentSessionOrigin::Subagent))
            .ok_or_else(|| {
                AgentError::InvalidInput(format!("Agent session {child} is not a subagent"))
            })?;
        let _admission = self
            .runs
            .admission
            .acquire()
            .await
            .map_err(|error| AgentError::Internal(error.to_string()))?;
        let selected = self.resolve(parent, &child.to_string()).await?;
        let name = selected.name.clone();
        let runs = self.runs.clone();
        let live = self
            .blocking(move |driver| runs.live_agents(driver, child))
            .await?;
        let mut tree = vec![(parent, selected)];
        for agent in self.agents(child).await? {
            if live.contains(&agent.id) {
                tree.push((child, agent));
            }
        }
        {
            let mut runs = self.runs.runs.lock();
            for (parent, agent) in &tree {
                let run = runs
                    .entry(agent.id)
                    .or_insert_with(|| AgentRun::new(*parent, agent.clone()));
                run.closing = true;
                run.turn += 1;
            }
        }
        let mut result = self.close_tree(&tree).await;
        let stopped = result.is_ok();
        if stopped {
            result = self
                .driver
                .enqueue_wake_event(
                    parent,
                    AgentWakeSource::Subagent {
                        session_id: child,
                        name,
                        event: AgentSubagentWake::ClosedByUser,
                    },
                )
                .await
                .map(|_| ());
        }
        for (_, agent) in &tree {
            let status = if stopped {
                AgentRunStatus::Closed
            } else if self.runs.has_work(&self.driver, agent.id) {
                AgentRunStatus::Waiting(None)
            } else {
                AgentRunStatus::Interrupted
            };
            if let Some(run) = self.runs.runs.lock().get_mut(&agent.id) {
                run.closing = false;
                run.queued.clear();
                run.set(status);
            }
            self.runs.feed.hint(agent.id);
            self.runs.signals.signal(agent.id);
        }
        self.runs.changed.notify_waiters();
        self.runs.signals.signal(parent);
        result
    }

    async fn close_tree(&self, tree: &[(AgentSessionId, ListedAgent)]) -> Result<(), AgentError> {
        for (_, agent) in tree.iter().rev() {
            self.stop_turn(agent.id).await?;
            let id = agent.id;
            self.blocking(move |driver| driver.quiesce_background_terminals(id))
                .await?;
            self.driver
                .take_wake_events(id, AgentWakeFilter::All)
                .await?;
        }
        Ok(())
    }
}
