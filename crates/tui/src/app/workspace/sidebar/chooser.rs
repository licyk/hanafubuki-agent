// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::sidebar_files::{FileEntry, FileTarget};
use crate::text_formatting::strip_terminal_controls;
use hanafubuki_agent::AgentSessionId;
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Stylize,
    text::Line,
    widgets::Widget,
};

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Choice {
    New,
    Chat(AgentSessionId),
    Agent(AgentSessionId),
    File(FileTarget),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FooterAction {
    Open,
    Path,
    Archive,
    Back,
    Hide,
}

struct Row {
    label: String,
    choice: Option<Choice>,
}

#[derive(Default)]
pub(super) struct Chooser {
    rows: Vec<Row>,
    selected: usize,
    top: usize,
    height: usize,
    hits: Vec<(usize, Rect)>,
    wheel_scrolled: bool,
    full_path: Option<(String, u16)>,
    footer: Vec<(FooterAction, Rect)>,
}

impl Chooser {
    pub(super) fn update(
        &mut self,
        parent: Option<AgentSessionId>,
        chats: &[(AgentSessionId, String)],
        saved: &[(AgentSessionId, String)],
        agents: &[(AgentSessionId, String)],
        files: &[FileEntry],
    ) {
        let selected = self.choice();
        let mut rows = vec![Row {
            choice: parent.map(|_| Choice::New),
            label: if parent.is_some() {
                "＋ New side chat"
            } else {
                "Send a message to start a side chat."
            }
            .into(),
        }];
        let mut group = |title: &str, entries: Vec<(Choice, String)>| {
            if entries.is_empty() {
                return;
            }
            rows.push(Row {
                choice: None,
                label: format!("{title}  {}", entries.len()),
            });
            rows.extend(entries.into_iter().map(|(choice, label)| Row {
                choice: Some(choice),
                label,
            }));
        };
        let mut seen = std::collections::HashSet::new();
        group(
            "Side chats",
            chats
                .iter()
                .chain(saved)
                .filter(|(id, _)| seen.insert(*id))
                .map(|(id, name)| (Choice::Chat(*id), name.clone()))
                .collect(),
        );
        group(
            "Subagents",
            agents
                .iter()
                .map(|(id, name)| (Choice::Agent(*id), name.clone()))
                .collect(),
        );
        group(
            "Files",
            files
                .iter()
                .map(|entry| (Choice::File(entry.target.clone()), entry.label.clone()))
                .collect(),
        );
        self.rows = rows;
        self.selected = selected
            .and_then(|choice| {
                self.rows
                    .iter()
                    .position(|row| row.choice.as_ref() == Some(&choice))
            })
            .or_else(|| self.rows.iter().position(|row| row.choice.is_some()))
            .unwrap_or(0);
    }

    pub(super) fn choice(&self) -> Option<Choice> {
        self.rows.get(self.selected)?.choice.clone()
    }
    pub(super) fn visible_choice(&self) -> Option<Choice> {
        (!self.wheel_scrolled || self.hits.iter().any(|(index, _)| *index == self.selected))
            .then(|| self.choice())
            .flatten()
    }

    pub(super) fn show_path(&mut self) {
        if let Some(Choice::File(FileTarget::Document { path, .. } | FileTarget::Directory(path))) =
            self.visible_choice()
        {
            self.full_path = Some((strip_terminal_controls(&path.to_string_lossy()), 0));
        }
    }

    pub(super) fn path_visible(&self) -> bool {
        self.full_path.is_some()
    }
    pub(super) fn dismiss_path(&mut self) {
        self.full_path = None;
    }

    pub(super) fn footer_at(&self, pos: Position) -> Option<FooterAction> {
        self.footer
            .iter()
            .find_map(|(action, rect)| rect.contains(pos).then_some(*action))
    }

    pub(super) fn at(&mut self, pos: Position) -> Option<Choice> {
        let index = self.hits.iter().find(|(_, r)| r.contains(pos))?.0;
        self.selected = index;
        self.choice()
    }

    pub(super) fn step(&mut self, delta: isize) {
        self.wheel_scrolled = false;
        let choices: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.choice.is_some())
            .map(|(i, _)| i)
            .collect();
        if choices.is_empty() {
            return;
        }
        let index = choices
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0);
        self.selected = choices[index.saturating_add_signed(delta).min(choices.len() - 1)];
    }

    pub(super) fn scroll(&mut self, delta: isize) {
        if let Some((_, top)) = &mut self.full_path {
            *top =
                top.saturating_add_signed(delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16);
            return;
        }
        self.wheel_scrolled = true;
        self.top = self
            .top
            .saturating_add_signed(delta)
            .min(self.rows.len().saturating_sub(self.height));
    }

    pub(super) fn page(&self) -> isize {
        self.height.max(1) as isize
    }

    pub(super) fn paint(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        pointer: Option<Position>,
        focused: bool,
        notice: Option<&str>,
    ) {
        self.hits.clear();
        self.footer.clear();
        if area.is_empty() {
            return;
        }
        let width = area.width.saturating_sub(2);
        let x = area.x.saturating_add(1);
        let line = |y| Rect::new(x, y, width, 1).intersection(area);
        Line::from("Open in sidebar")
            .bold()
            .render(line(area.y), buf);
        if let Some((path, top)) = &mut self.full_path {
            use ratatui::widgets::{Paragraph, Wrap};
            let body = Rect::new(
                x,
                area.y.saturating_add(2),
                width,
                area.height.saturating_sub(3),
            );
            let paragraph = Paragraph::new(path.as_str()).wrap(Wrap { trim: false });
            *top = (*top).min(
                paragraph
                    .line_count(width)
                    .saturating_sub(usize::from(body.height))
                    .min(u16::MAX as usize) as u16,
            );
            paragraph.scroll((*top, 0)).render(body, buf);
            let rect = line(area.bottom().saturating_sub(1));
            Line::from("Esc back").dim().render(rect, buf);
            self.footer.push((FooterAction::Back, rect));
            return;
        }
        let mut start = area.y.saturating_add(2);
        if let Some(message) = notice {
            Line::from(strip_terminal_controls(message))
                .dim()
                .render(line(start.saturating_sub(1)), buf);
        }
        start = start.min(area.bottom());
        let end = area.bottom().saturating_sub(1);
        self.height = usize::from(end.saturating_sub(start));
        if !self.wheel_scrolled {
            if self.selected < self.top {
                self.top = self.selected
            } else if self.selected >= self.top + self.height {
                self.top = (self.selected + 1).saturating_sub(self.height);
            }
        }
        self.top = self.top.min(self.rows.len().saturating_sub(self.height));
        for (row_index, y) in (self.top..self.rows.len()).zip(start..end) {
            let row = &self.rows[row_index];
            let rect = line(y);
            let selected = row.choice.is_some() && self.selected == row_index;
            let marker = if selected { "› " } else { "  " };
            let label = strip_terminal_controls(&row.label);
            let mut label = if row.choice.is_none() {
                Line::from(label).dim()
            } else {
                Line::from(format!("{marker}{label}"))
            };
            if row.choice.is_some() {
                self.hits.push((row_index, rect));
                if selected && focused {
                    label = label.reversed()
                } else if pointer.is_some_and(|p| rect.contains(p)) {
                    label = label.underlined()
                }
            }
            truncate_line_with_ellipsis_if_overflow(label, usize::from(rect.width))
                .render(rect, buf);
        }
        if area.height > 2 {
            let mut left = x;
            let choice = self.visible_choice();
            for (action, label, visible) in [
                (FooterAction::Open, "Enter open", choice.is_some()),
                (
                    FooterAction::Path,
                    "p path",
                    matches!(
                        choice,
                        Some(Choice::File(
                            FileTarget::Document { .. } | FileTarget::Directory(_)
                        ))
                    ),
                ),
                (
                    FooterAction::Archive,
                    "Del archive",
                    matches!(choice, Some(Choice::Chat(_))),
                ),
                (FooterAction::Back, "Esc back", true),
                (FooterAction::Hide, "hide", true),
            ] {
                if !visible || left >= area.right() {
                    continue;
                }
                let rect = Rect::new(left, end, (label.len() as u16).min(area.right() - left), 1);
                Line::from(label).dim().render(rect, buf);
                self.footer.push((action, rect));
                left = rect.right().saturating_add(2);
            }
        }
    }
}
