// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use super::Choice;
use crate::agent_bridge::NavigationAgent;
use crate::line_truncation::{
    filename_label, middle_label, truncate_line_with_ellipsis_if_overflow,
};
use crate::sidebar_files::{FileEntry, FileTarget, SidebarFiles};
use crate::text_formatting::strip_terminal_controls;
use crate::wrapping::display_width;
use hanafubuki_agent::{AgentSessionId, AgentSubagentState};
use ratatui::{
    style::{Style, Styled},
    text::Line,
};
use std::ops::Range;
use std::path::Path;

pub(super) struct Entry {
    pub(super) choice: Choice,
    pub(super) detail: Option<String>,
    pub(super) header: Option<String>,
    pub(super) lines: Range<usize>,
    symbol: &'static str,
    title: String,
}

impl Entry {
    fn new(choice: Choice, symbol: &'static str, title: &str, detail: Option<&str>) -> Self {
        Self {
            choice,
            symbol,
            title: strip_terminal_controls(title),
            detail: detail.map(strip_terminal_controls),
            header: None,
            lines: 0..0,
        }
    }

    pub(super) fn new_chat(available: bool) -> Self {
        let description = if available {
            "Start from this conversation"
        } else {
            "Available after first message"
        };
        Self::new(Choice::New, "+", "New side chat", Some(description))
    }

    pub(super) fn chat(id: AgentSessionId, title: &str, prompt: Option<&str>) -> Self {
        Self::new(Choice::Chat(id), "○", title, prompt)
    }

    pub(super) fn agent(agent: &NavigationAgent) -> Self {
        let (symbol, status) = match agent.status {
            AgentSubagentState::Running => ("●", "Running"),
            AgentSubagentState::Waiting => ("●", "Waiting"),
            AgentSubagentState::Failed => ("!", "Failed"),
            AgentSubagentState::Idle => ("○", "Idle"),
            AgentSubagentState::Closed => ("○", "Closed"),
        };
        Self::new(
            Choice::Agent(agent.session_id),
            symbol,
            &agent.title,
            Some(status),
        )
    }

    pub(super) fn file(entry: &FileEntry, files: &SidebarFiles) -> Self {
        let choice = Choice::File(entry.target.clone());
        let path = match &entry.target {
            FileTarget::Roots => {
                return Self::new(choice, "·", &entry.label, Some("Workspace roots"))
            }
            FileTarget::Directory(path) | FileTarget::Document { path, .. } => path,
        };
        let relative = files.relative_path(path);
        let directory = if entry.label == "../" {
            Some(relative.as_path())
        } else {
            relative.parent()
        }
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("Project root"));
        if matches!(&entry.target, FileTarget::Directory(_))
            && entry.label != "../"
            && relative.as_os_str().is_empty()
        {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            return Self::new(
                choice,
                "·",
                &format!("Browse {name}/"),
                Some(&path.to_string_lossy()),
            );
        }
        let name = if matches!(entry.target, FileTarget::Document { .. }) {
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
        } else {
            entry.label.as_str().into()
        };
        Self::new(choice, "·", &name, Some(&directory.to_string_lossy()))
    }

    pub(super) fn group(entries: &mut [Self], start: usize, name: &str) {
        let count = entries.len() - start;
        if let Some(entry) = entries.get_mut(start) {
            entry.header = Some(format!("{name}  {count}"));
        }
    }

    pub(super) fn line(
        &self,
        first: bool,
        wide: bool,
        width: usize,
        selected: bool,
        fill: Style,
    ) -> Line<'static> {
        let secondary = if fill == Style::default() {
            fill.dim()
        } else {
            fill.not_bold()
        };
        let detail = self.detail.as_deref().unwrap_or_default();
        if !first {
            let text = if matches!(self.choice, Choice::File(_)) {
                middle_label(detail, width.saturating_sub(4))
            } else {
                detail.to_owned()
            };
            return truncate_line_with_ellipsis_if_overflow(
                Line::from(format!("    {text}")).style(secondary),
                width,
            );
        }
        let marker = if selected { '›' } else { ' ' };
        if matches!(self.choice, Choice::File(_)) {
            let budget = width.saturating_sub(4);
            let name = filename_label(
                &self.title,
                if wide {
                    budget.saturating_sub(5)
                } else {
                    budget
                },
            );
            let mut line = Line::from(format!("{marker} · {name}"));
            if wide {
                let directory =
                    middle_label(detail, budget.saturating_sub(display_width(&name) + 2));
                line.spans
                    .push(format!("  {directory}").set_style(secondary));
            }
            return line.style(fill);
        }
        let title = Line::from(format!("{marker} {} {}", self.symbol, self.title));
        if wide && !matches!(self.choice, Choice::Chat(_)) {
            let mut line = truncate_line_with_ellipsis_if_overflow(
                title,
                width.saturating_sub(display_width(detail) + 2),
            );
            let pad = width.saturating_sub(line.width() + display_width(detail));
            line.spans
                .push(format!("{:pad$}{detail}", "").set_style(secondary));
            line.style(fill)
        } else {
            truncate_line_with_ellipsis_if_overflow(title, width).style(fill)
        }
    }
}
