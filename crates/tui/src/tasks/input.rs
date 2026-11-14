use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use super::{Confirmation, Section, TaskAction, Tasks};

impl Tasks {
    pub(crate) fn key(&mut self, key: KeyEvent) -> Option<TaskAction> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if let Some(confirm) = &mut self.confirmation {
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') => self.confirmation = None,
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    confirm.accepted = !confirm.accepted
                }
                KeyCode::Char('y') => return self.confirmation.take().map(|c| c.action),
                KeyCode::Enter if confirm.accepted => {
                    return self.confirmation.take().map(|c| c.action)
                }
                KeyCode::Enter => self.confirmation = None,
                _ => {}
            }
            return None;
        }
        let selected = self.selected;
        match key.code {
            KeyCode::Esc => return Some(TaskAction::Leave),
            KeyCode::Tab | KeyCode::BackTab => {
                self.section = if self.section == Section::Agents {
                    Section::Terminals
                } else {
                    Section::Agents
                };
                self.selected = 0;
                self.detail_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.len().saturating_sub(1))
            }
            KeyCode::PageDown => self.detail_scroll = self.detail_scroll.saturating_add(10),
            KeyCode::PageUp => self.detail_scroll = self.detail_scroll.saturating_sub(10),
            KeyCode::Char('r')
                if self.view.as_ref().is_some_and(|view| {
                    matches!(
                        view.tasks.wake,
                        hanafubuki_agent::AgentWakeState::Paused { .. }
                    )
                }) =>
            {
                return Some(TaskAction::Resume)
            }
            KeyCode::Enter if self.section == Section::Agents => {
                return self
                    .view
                    .as_ref()?
                    .tasks
                    .agents
                    .get(self.selected)
                    .map(|agent| TaskAction::Open(agent.session_id));
            }
            KeyCode::Enter => {
                self.expanded = !self.expanded;
                self.detail_scroll = 0;
            }
            KeyCode::Char('x') => self.confirm_close(),
            KeyCode::Char('X') if self.section == Section::Terminals => {
                self.confirmation = Some(Confirmation {
                    action: TaskAction::CloseTerminals,
                    question: "Stop every terminal of this conversation? Its agent is told.".into(),
                    accepted: false,
                });
            }
            _ => {}
        }
        if self.selected != selected {
            self.detail_scroll = 0;
            self.expanded = false;
        }
        None
    }

    fn confirm_close(&mut self) {
        let Some(view) = &self.view else {
            return;
        };
        let (action, question) = match self.section {
            Section::Agents => {
                let Some(agent) = view.tasks.agents.get(self.selected) else {
                    return;
                };
                let children = view
                    .tasks
                    .agents
                    .iter()
                    .filter(|child| child.parent_session_id == agent.session_id)
                    .count();
                (
                    TaskAction::CloseAgent(agent.session_id),
                    format!(
                        "Close {} and its {children} sub-agents? Its parent is told it was closed.",
                        agent.title
                    ),
                )
            }
            Section::Terminals => {
                let Some(terminal) = view.tasks.terminals.get(self.selected) else {
                    return;
                };
                (
                    TaskAction::CloseTerminal(terminal.clone()),
                    format!(
                        "Close terminal #{}? Its agent is told it was stopped.",
                        terminal.terminal.execution_id.value()
                    ),
                )
            }
        };
        self.confirmation = Some(Confirmation {
            action,
            question,
            accepted: false,
        });
    }
}
