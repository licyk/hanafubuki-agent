use std::collections::BTreeMap;

use hanafubuki_agent::{
    AgentHandle, AgentHistorySortDirection, AgentSessionId, AgentTranscriptPageRequest,
    ConversationItem,
};

use super::AgentBridge;
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};
use crate::tasks::{AgentDetails, TaskAction, TaskView, TasksEvent};

impl AgentBridge {
    pub(crate) fn read_tasks(&self, root: AgentSessionId, sender: AppEventSender) {
        let agent = self.agent.clone();
        std::thread::spawn(move || {
            let result = read(&agent, root);
            sender.send(AppEvent::Workspace(WorkspaceEvent::Tasks(Box::new(
                TasksEvent::Read { root, result },
            ))));
        });
    }

    pub(crate) fn task_action(
        &self,
        root: AgentSessionId,
        action: TaskAction,
        sender: AppEventSender,
    ) {
        let agent = self.agent.clone();
        std::thread::spawn(move || {
            let worker = agent.clone();
            let result = match action {
                TaskAction::CloseAgent(id) => agent
                    .runtime()
                    .block_on(async move { worker.close_subagent(id).await })
                    .map_err(|error| hanafubuki_agent::AgentError::Internal(error.to_string()))
                    .and_then(|result| result),
                TaskAction::CloseTerminal(terminal) => {
                    agent.close_background_terminal(terminal.owner, terminal.terminal.execution_id)
                }
                TaskAction::CloseTerminals => agent
                    .stop_background_terminals_confirmed(root)
                    .and_then(|outcome| {
                        if outcome.retained.is_empty() {
                            Ok(())
                        } else {
                            Err(hanafubuki_agent::AgentError::Internal(format!(
                                "{} terminals could not be stopped",
                                outcome.retained.len()
                            )))
                        }
                    }),
                TaskAction::Resume => agent.resume_background_wakes(root),
                TaskAction::Leave | TaskAction::Open(_) => return,
            }
            .map_err(|error| error.to_string());
            sender.send(AppEvent::Workspace(WorkspaceEvent::Tasks(Box::new(
                TasksEvent::Done { root, result },
            ))));
        });
    }
}

fn read(agent: &AgentHandle, root: AgentSessionId) -> Result<TaskView, String> {
    let reader = agent.clone();
    let tasks = agent
        .runtime()
        .block_on(async move { reader.background_tasks(root).await })
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let mut details = BTreeMap::new();
    for child in &tasks.agents {
        let role = agent
            .snapshot(child.session_id)
            .ok()
            .and_then(|snapshot| snapshot.role)
            .unwrap_or_else(|| "default".into());
        let task = first_task(agent, child.session_id);
        let work = agent
            .background_status(child.session_id)
            .ok()
            .map(|status| status.work);
        details.insert(child.session_id, AgentDetails { task, role, work });
    }
    Ok(TaskView { tasks, details })
}

fn first_task(agent: &AgentHandle, child: AgentSessionId) -> String {
    let mut request = AgentTranscriptPageRequest {
        session_id: child,
        page_size: 64,
        cursor: None,
        direction: AgentHistorySortDirection::Ascending,
        through_revision: None,
    };
    while let Ok(page) = agent.read_transcript_page(request.clone()) {
        // A fork keeps inherited items' source ids. Its assigned task is
        // the first user message written by this child, possibly pages later.
        if let Some(task) = page.items.into_iter().find_map(|item| match item.content {
            ConversationItem::UserMessage { text, .. } if item.source_session_id == child => {
                Some(text)
            }
            _ => None,
        }) {
            return task;
        }
        let Some(cursor) = page.newer_cursor else {
            break;
        };
        request.cursor = Some(cursor);
        request.through_revision = Some(page.through_revision);
    }
    String::new()
}
