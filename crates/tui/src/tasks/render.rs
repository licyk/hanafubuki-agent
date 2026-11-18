use hanafubuki_agent::{AgentSubagentState, AgentWakePause, AgentWakeState};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Stylize,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use super::{Section, Tasks};

impl Tasks {
    pub(crate) fn render(&self, area: Rect, buffer: &mut Buffer, now: u128) {
        self.hits.borrow_mut().clear();
        Clear.render(area, buffer);
        let areas = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);
        let title = if self.loading {
            "Background tasks · Loading…"
        } else {
            "Background tasks"
        };
        Paragraph::new(title.bold()).render(areas[0], buffer);
        let mut tabs =
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1).intersection(areas[0]);
        for (section, label, shortcut) in [
            (Section::Agents, "[A] Agents", 'a'),
            (Section::Terminals, "[T] Terminals", 't'),
        ] {
            let width = tabs.width.min(label.len() as u16);
            let tab = Rect { width, ..tabs };
            let label = if self.section == section {
                label.cyan().bold().reversed()
            } else {
                label.dim()
            };
            Paragraph::new(label).render(tab, buffer);
            self.hits.borrow_mut().push((
                tab,
                super::Target::Key(ratatui::crossterm::event::KeyCode::Char(shortcut)),
            ));
            tabs.x = tabs.x.saturating_add(width + 1);
            tabs.width = tabs.width.saturating_sub(width + 1);
        }
        let body = Layout::default()
            .direction(if area.width >= 90 {
                Direction::Horizontal
            } else {
                Direction::Vertical
            })
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(areas[1]);
        let selected_section = match self.section {
            Section::Agents => "Agents",
            Section::Terminals => "Terminals",
        };
        let mut lines: Vec<Line> = vec![format!(
            "{selected_section} · Tab switches section · {}/{}",
            usize::from(self.len() > 0) + self.selected,
            self.len()
        )
        .bold()
        .into()];
        let mut rows = Vec::new();
        let mut selected_start = 0;
        let mut selected_end = 0;
        if let Some(view) = &self.view {
            match self.section {
                Section::Agents => {
                    for (index, agent) in view.tasks.agents.iter().enumerate() {
                        rows.push((lines.len(), index));
                        if index == self.selected {
                            selected_start = lines.len();
                        }
                        let marker = if self.selected == index { "›" } else { " " };
                        let indent = "  ".repeat(agent.depth.saturating_sub(1));
                        let elapsed = agent
                            .state_since
                            .filter(|_| {
                                matches!(
                                    agent.status,
                                    AgentSubagentState::Running | AgentSubagentState::Waiting
                                )
                            })
                            .map(|start| {
                                format!(" · {}", elapsed(now.saturating_sub(u128::from(start))))
                            })
                            .unwrap_or_default();
                        let state = match agent.status {
                            AgentSubagentState::Running => "running",
                            AgentSubagentState::Waiting => "waiting",
                            AgentSubagentState::Idle => "idle",
                            AgentSubagentState::Failed => "failed",
                            AgentSubagentState::Closed => "closed",
                        };
                        let state = if agent.status == AgentSubagentState::Waiting {
                            view.details
                                .get(&agent.session_id)
                                .and_then(|detail| detail.work)
                                .map_or_else(
                                    || state.into(),
                                    |work| {
                                        format!(
                                            "waiting ({} {} · {} {})",
                                            work.agents,
                                            if work.agents == 1 { "agent" } else { "agents" },
                                            work.terminals,
                                            if work.terminals == 1 {
                                                "terminal"
                                            } else {
                                                "terminals"
                                            }
                                        )
                                    },
                                )
                        } else {
                            state.into()
                        };
                        let role = view
                            .details
                            .get(&agent.session_id)
                            .map_or("default", |detail| detail.role.as_str());
                        let line = format!(
                            "{marker} {indent}{}  {state}{elapsed} · {} · {role}",
                            agent.title, agent.model
                        );
                        lines.push(if self.selected == index {
                            line.cyan().into()
                        } else {
                            line.into()
                        });
                        if index == self.selected {
                            selected_end = lines.len();
                        }
                    }
                }
                Section::Terminals => {
                    for (index, terminal) in view.tasks.terminals.iter().enumerate() {
                        rows.push((lines.len(), index));
                        if index == self.selected {
                            selected_start = lines.len();
                        }
                        let t = &terminal.terminal;
                        let state = format!("{:?}", t.status).to_lowercase();
                        let marker = if self.selected == index { "›" } else { " " };
                        let owner = terminal
                            .owner_title
                            .clone()
                            .unwrap_or_else(|| terminal.owner.to_string());
                        lines.push(
                            format!(
                                "{marker} #{} {} · {state} · {} · {owner} · {}",
                                t.execution_id.value(),
                                t.command,
                                elapsed(now.saturating_sub(t.created_at_ms)),
                                t.cwd
                            )
                            .into(),
                        );
                        for output in t
                            .recent_output
                            .iter()
                            .rev()
                            .take(3)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                        {
                            lines.push(format!("    {output}").dim().into());
                        }
                        if index == self.selected {
                            selected_end = lines.len();
                        }
                    }
                }
            }
        }
        if self.len() == 0 {
            lines.push("None".dim().into());
        }
        let scroll = selected_end
            .saturating_sub(usize::from(body[0].height))
            .min(selected_start);
        for y in 0..body[0].height {
            let line = scroll + usize::from(y);
            if line < lines.len() {
                if let Some((_, index)) = rows.iter().rev().find(|(start, _)| *start <= line) {
                    self.hits.borrow_mut().push((
                        Rect::new(body[0].x, body[0].y + y, body[0].width, 1),
                        super::Target::Row(*index),
                    ));
                }
            }
        }
        self.hits
            .borrow_mut()
            .push((body[1], super::Target::Details));
        Paragraph::new(lines)
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
            .render(body[0], buffer);
        Paragraph::new(self.details())
            .wrap(Wrap { trim: false })
            .scroll((self.detail_scroll, 0))
            .block(Block::default().borders(Borders::LEFT).title(" Details "))
            .render(body[1], buffer);
        self.render_actions(areas[2], buffer);
        self.render_confirmation(area, buffer);
    }

    fn render_actions(&self, area: Rect, buffer: &mut Buffer) {
        Paragraph::new(self.error.clone().unwrap_or_else(|| self.wake_line())).render(area, buffer);
        let mut x = area.x;
        let mut y = area.y.saturating_add(1);
        use ratatui::crossterm::event::KeyCode;
        for (label, key) in [
            ("Enter inspect", KeyCode::Enter),
            ("x close", KeyCode::Char('x')),
            ("X stop terminals", KeyCode::Char('X')),
            ("r resume", KeyCode::Char('r')),
            ("Esc return", KeyCode::Esc),
        ] {
            let width = label.len() as u16;
            if x > area.x && x.saturating_add(width) > area.right() {
                x = area.x;
                y = y.saturating_add(1);
            }
            let rect = Rect::new(x, y, width, 1).intersection(area);
            Paragraph::new(label).render(rect, buffer);
            self.hits.borrow_mut().push((rect, super::Target::Key(key)));
            x = x.saturating_add(width + 3);
        }
    }

    fn render_confirmation(&self, area: Rect, buffer: &mut Buffer) {
        if let Some(confirm) = &self.confirmation {
            let width = area.width.min(72);
            let height = area.height.min(7);
            let modal = Rect::new(
                area.x + (area.width - width) / 2,
                area.y + (area.height - height) / 2,
                width,
                height,
            );
            Clear.render(modal, buffer);
            let block = Block::bordered().title(" Confirm ");
            let inner = block.inner(modal);
            block.render(modal, buffer);
            let [question, choices, help] = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(inner);
            Paragraph::new(confirm.question.as_str())
                .wrap(Wrap { trim: false })
                .render(question, buffer);
            self.hits.borrow_mut().clear();
            let [no, yes] =
                Layout::horizontal([Constraint::Max(21), Constraint::Min(0)]).areas(choices);
            for (rect, label, active, key) in [
                (no, "No, keep running", !confirm.accepted, 'n'),
                (yes, "Yes, close", confirm.accepted, 'y'),
            ] {
                let marker = if active { "› " } else { "  " };
                Paragraph::new(format!("{marker}{label}")).render(rect, buffer);
                self.hits.borrow_mut().push((
                    rect,
                    super::Target::Key(ratatui::crossterm::event::KeyCode::Char(key)),
                ));
            }
            Paragraph::new("Tab chooses · Enter confirms · Esc cancels".dim()).render(help, buffer);
        }
    }

    fn details(&self) -> Vec<Line<'static>> {
        let Some(view) = &self.view else {
            return vec!["Reading task details…".into()];
        };
        match self.section {
            Section::Agents => {
                let Some(agent) = view.tasks.agents.get(self.selected) else {
                    return vec![];
                };
                let detail = view.details.get(&agent.session_id);
                let own = view
                    .tasks
                    .terminals
                    .iter()
                    .filter(|terminal| terminal.owner == agent.session_id)
                    .count();
                let mut lines = vec![
                    agent.title.clone().bold().into(),
                    format!(
                        "{} · {} · {}",
                        agent.provider_id,
                        agent.model,
                        agent
                            .reasoning_effort
                            .as_ref()
                            .map_or("default", |e| e.as_str())
                    )
                    .into(),
                    format!(
                        "Role: {} · {own} terminals",
                        detail.map_or("default", |d| d.role.as_str())
                    )
                    .into(),
                    "".into(),
                    detail.map_or_else(|| "".into(), |d| d.task.clone()).into(),
                    "".into(),
                    agent
                        .last_answer
                        .clone()
                        .unwrap_or_else(|| "No answer yet.".into())
                        .into(),
                ];
                lines.extend(
                    view.tasks
                        .terminals
                        .iter()
                        .filter(|terminal| terminal.owner == agent.session_id)
                        .map(|terminal| {
                            Line::from(format!(
                                "#{} {}",
                                terminal.terminal.execution_id.value(),
                                terminal.terminal.command
                            ))
                        }),
                );
                lines
            }
            Section::Terminals => {
                let Some(terminal) = view.tasks.terminals.get(self.selected) else {
                    return vec![];
                };
                let t = &terminal.terminal;
                let mut lines = vec![
                    t.command.clone().bold().into(),
                    format!("in {} · exit {:?}", t.cwd, t.exit_code).into(),
                    "".into(),
                ];
                let skip = if self.expanded {
                    0
                } else {
                    t.recent_output.len().saturating_sub(3)
                };
                lines.extend(t.recent_output.iter().skip(skip).cloned().map(Line::from));
                lines.push(
                    if self.expanded {
                        "− Hide details (Enter)"
                    } else {
                        "+ Show details (Enter)"
                    }
                    .dim()
                    .into(),
                );
                lines
            }
        }
    }

    fn wake_line(&self) -> String {
        match self.view.as_ref().map(|view| view.tasks.wake) {
            Some(AgentWakeState::On) => "wake: on".into(),
            Some(AgentWakeState::Off) => "wake: off (wakeOnBackgroundTasks)".into(),
            Some(AgentWakeState::Paused { reason }) => format!(
                "wake: paused ({}) · r resumes",
                match reason {
                    AgentWakePause::PersonInterrupted => "you interrupted",
                    AgentWakePause::RunawayBound => "wake limit reached",
                }
            ),
            None => "wake: loading".into(),
        }
    }
}

fn elapsed(ms: u128) -> String {
    let seconds = ms / 1000;
    format!("{}m{:02}s", seconds / 60, seconds % 60)
}
