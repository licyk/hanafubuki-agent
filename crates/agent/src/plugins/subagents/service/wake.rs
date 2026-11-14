//! Resuming a child and holding its answer until its background work is quiet.

use hanafubuki_agent_foundation::panic::catch_panic_async;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId, AgentSessionOrigin};

use super::runs::AgentRun;
use super::{AgentRunStatus, SubagentRuns, SubagentService};
use crate::session_driver::AgentSessionDriver;
use crate::wake::root_of;

impl SubagentService {
    pub(crate) async fn resume(&self, child: AgentSessionId) -> Result<(), AgentError> {
        let session = self.session(child).await?;
        let Some(parent) = session
            .parent_session_id
            .filter(|_| session.origin == Some(AgentSessionOrigin::Subagent))
        else {
            return Ok(());
        };
        let root = self.blocking(move |driver| root_of(driver, child)).await?;
        if self.wakes.pauses.pause(root).is_some() {
            return Ok(());
        }
        let agent = self.resolve(parent, &child.to_string()).await?;
        let (turn, previous) = {
            let mut runs = self.runs.runs.lock();
            let run = runs
                .entry(child)
                .or_insert_with(|| AgentRun::new(parent, agent));
            if run.closing || matches!(run.status, AgentRunStatus::Running | AgentRunStatus::Closed)
            {
                return Ok(());
            }
            let previous = run.status.clone();
            run.turn += 1;
            run.set(AgentRunStatus::Running);
            (run.turn, previous)
        };
        self.runs.feed.hint(child);
        let driver = self.driver.clone();
        let runs = self.runs.clone();
        let events = self.wakes.woken_child_events(root);
        tokio::spawn(async move {
            let outcome = catch_panic_async(driver.run_wake_turn(child, events)).await;
            let status = match outcome {
                Ok(Ok(Some(outcome))) => AgentRunStatus::Completed(Some(outcome.final_answer)),
                Ok(Ok(None)) => {
                    let mut records = runs.runs.lock();
                    if let Some(run) = records.get_mut(&child).filter(|run| run.turn == turn) {
                        run.set(previous);
                    }
                    runs.feed.hint(child);
                    return;
                }
                Ok(Err(error)) => AgentRunStatus::of_turn(Ok(Err(error))),
                Err(error) => AgentRunStatus::of_turn(Err(error)),
            };
            runs.finish(&driver, child, turn, status).await;
        });
        Ok(())
    }

    pub(crate) async fn settle(&self, child: AgentSessionId) {
        self.runs.settle(&self.driver, child).await;
    }
}

impl SubagentRuns {
    pub(super) fn has_work(&self, driver: &AgentSessionDriver, child: AgentSessionId) -> bool {
        self.running(child) > 0
            || !driver.background_work(child).is_empty()
            || driver
                .loaded_snapshot(child)
                .is_ok_and(|session| session.pending_wake_events > 0)
    }

    pub(super) async fn settle(&self, driver: &AgentSessionDriver, child: AgentSessionId) {
        if driver
            .loaded_snapshot(child)
            .is_ok_and(|session| session.turn.is_some_and(|turn| !turn.phase.is_terminal()))
            || self.has_work(driver, child)
        {
            return;
        }
        let held = self.runs.lock().get(&child).and_then(|run| {
            if let AgentRunStatus::Waiting(answer) = &run.status {
                if run.closing {
                    return None;
                }
                Some((run.turn, answer.clone()))
            } else {
                None
            }
        });
        if let Some((turn, answer)) = held {
            self.finish(driver, child, turn, AgentRunStatus::Completed(answer))
                .await;
        }
    }
}
