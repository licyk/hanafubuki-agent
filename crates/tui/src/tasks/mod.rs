//! The background task manager over the Agent's task snapshot and change feed.

mod input;
mod render;

use std::collections::BTreeMap;

use hanafubuki_agent::{AgentBackgroundTasks, AgentBackgroundTerminal, AgentSessionId};

#[derive(Clone, Debug)]
pub(crate) enum TaskAction {
    Leave,
    Open(AgentSessionId),
    CloseAgent(AgentSessionId),
    CloseTerminal(AgentBackgroundTerminal),
    CloseTerminals,
    Resume,
}

#[derive(Debug)]
pub(crate) enum TasksEvent {
    Open,
    Read {
        root: AgentSessionId,
        result: Result<TaskView, String>,
    },
    Done {
        root: AgentSessionId,
        result: Result<(), String>,
    },
}

#[derive(Debug)]
pub(crate) struct TaskView {
    pub(crate) tasks: AgentBackgroundTasks,
    pub(crate) details: BTreeMap<AgentSessionId, AgentDetails>,
}

#[derive(Debug, Default)]
pub(crate) struct AgentDetails {
    pub(crate) task: String,
    pub(crate) role: String,
    pub(crate) work: Option<hanafubuki_agent::AgentBackgroundWork>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Agents,
    Terminals,
}

struct Confirmation {
    action: TaskAction,
    question: String,
    accepted: bool,
}

pub(crate) struct Tasks {
    pub(crate) root: AgentSessionId,
    pub(crate) view: Option<TaskView>,
    section: Section,
    selected: usize,
    expanded: bool,
    detail_scroll: u16,
    confirmation: Option<Confirmation>,
    pub(crate) loading: bool,
    pub(crate) refresh_pending: bool,
    pub(crate) error: Option<String>,
}

impl Tasks {
    pub(crate) fn new(root: AgentSessionId) -> Self {
        Self {
            root,
            view: None,
            section: Section::Agents,
            selected: 0,
            expanded: false,
            detail_scroll: 0,
            confirmation: None,
            loading: false,
            refresh_pending: false,
            error: None,
        }
    }

    pub(crate) fn ingest(&mut self, result: Result<TaskView, String>) {
        self.loading = false;
        match result {
            Ok(view) => {
                self.view = Some(view);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        self.selected = self.selected.min(self.len().saturating_sub(1));
    }

    fn len(&self) -> usize {
        self.view.as_ref().map_or(0, |view| match self.section {
            Section::Agents => view.tasks.agents.len(),
            Section::Terminals => view.tasks.terminals.len(),
        })
    }
}

#[cfg(test)]
mod tests;
