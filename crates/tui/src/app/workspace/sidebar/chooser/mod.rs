// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

mod chrome;
mod rows;

use crate::agent_bridge::NavigationAgent;
use crate::sidebar_files::{FileTarget, SidebarFiles};
use crate::style::selection_style;
use hanafubuki_agent::AgentSessionId;
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Style, Stylize},
    text::Line,
    widgets::Widget,
};
use rows::Entry;
use std::collections::{HashMap, HashSet};

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Choice {
    New,
    Main,
    Chat(AgentSessionId),
    Agent(AgentSessionId),
    File(FileTarget),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FooterAction {
    Help,
    Open,
    Path,
    Archive,
    Back,
    Hide,
}

#[derive(Default)]
pub(super) struct Chooser {
    entries: Vec<Entry>,
    selected: usize,
    top: usize,
    height: usize,
    hits: Vec<(usize, Rect)>,
    wheel_scrolled: bool,
    can_start: bool,
    full_path: Option<(String, usize)>,
    footer: Vec<(FooterAction, Rect)>,
}

impl Chooser {
    pub(super) fn update(
        &mut self,
        parent: Option<AgentSessionId>,
        chats: &[(AgentSessionId, String)],
        saved: &[(AgentSessionId, String)],
        agents: &[NavigationAgent],
        files: &SidebarFiles,
        prompts: &HashMap<AgentSessionId, Option<String>>,
    ) {
        let selected = self.choice();
        self.can_start = parent.is_some();
        let mut entries = vec![Entry::new_chat(self.can_start)];
        if parent.is_some() {
            entries.push(Entry::main_agent());
        }
        let mut seen = HashSet::new();
        let start = entries.len();
        let chats: Vec<_> = chats
            .iter()
            .chain(saved)
            .filter(|(id, _)| seen.insert(*id))
            .collect();
        for (ordinal, (id, name)) in chats.iter().copied().enumerate() {
            let title = super::chat_label(name, ordinal, chats.len() > 1, "Side chat");
            entries.push(Entry::chat(
                *id,
                &title,
                prompts.get(id).and_then(Option::as_deref),
            ));
        }
        Entry::group(&mut entries, start, "Side chats");
        let start = entries.len();
        entries.extend(agents.iter().map(Entry::agent));
        Entry::group(&mut entries, start, "Subagents");
        let start = entries.len();
        entries.extend(
            files
                .entries()
                .iter()
                .map(|entry| Entry::file(entry, files)),
        );
        let group = if files.entries().iter().all(|entry| entry.is_markdown()) {
            "Markdown files"
        } else {
            "Files"
        };
        Entry::group(&mut entries, start, group);
        self.entries = entries;
        self.selected = selected
            .and_then(|choice| self.entries.iter().position(|e| e.choice == choice))
            .unwrap_or(0);
        self.hits.clear();
    }

    pub(super) fn choice(&self) -> Option<Choice> {
        self.entries
            .get(self.selected)
            .map(|row| row.choice.clone())
    }

    fn enabled(&self, index: usize) -> bool {
        self.can_start || !matches!(self.entries[index].choice, Choice::New | Choice::Chat(_))
    }

    pub(super) fn visible_choice(&self) -> Option<Choice> {
        let choice = self.choice()?;
        (self.enabled(self.selected)
            && (!self.wheel_scrolled || self.hits.iter().any(|(i, _)| *i == self.selected)))
        .then_some(choice)
    }

    pub(in crate::app) fn visible_chat_ids(&self) -> impl Iterator<Item = AgentSessionId> + '_ {
        self.hits
            .iter()
            .filter_map(|(index, _)| match self.entries[*index].choice {
                Choice::Chat(id) => Some(id),
                _ => None,
            })
    }

    pub(super) fn show_path(&mut self) {
        if let Some(Choice::File(FileTarget::Document { path, .. } | FileTarget::Directory(path))) =
            self.visible_choice()
        {
            self.full_path = Some((path.to_string_lossy().into_owned(), 0));
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
            .find_map(|(a, r)| r.contains(pos).then_some(*a))
    }
    pub(super) fn at(&mut self, pos: Position) -> Option<Choice> {
        self.selected = self.hits.iter().find(|(_, rect)| rect.contains(pos))?.0;
        self.choice()
    }

    pub(super) fn step(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let row = &self.entries[self.selected].lines;
        let next = match delta {
            isize::MIN => 0,
            isize::MAX => self.entries.len() - 1,
            d if d < 0 && row.start >= 1 + self.top + self.height => {
                self.hits.last().map_or(self.selected, |(i, _)| *i)
            }
            d if d > 0 && row.end <= 1 + self.top => {
                self.hits.first().map_or(self.selected, |(i, _)| *i)
            }
            d if d < -1 => self
                .entries
                .partition_point(|e| e.lines.end <= row.start.saturating_sub(self.height))
                .min(self.selected.saturating_sub(1)),
            d if d > 1 => self
                .entries
                .partition_point(|e| e.lines.start <= row.start + self.height)
                .saturating_sub(1)
                .max((self.selected + 1).min(self.entries.len() - 1)),
            _ => self
                .selected
                .saturating_add_signed(delta)
                .min(self.entries.len() - 1),
        };
        self.selected = next;
        self.wheel_scrolled = false;
        if next == 0 {
            self.top = 0;
        }
    }
    pub(super) fn scroll(&mut self, delta: isize) {
        if let Some((_, top)) = &mut self.full_path {
            *top = top.saturating_add_signed(delta);
            return;
        }
        self.wheel_scrolled = true;
        self.top = self.top.saturating_add_signed(delta).min(self.max_top());
    }
    fn max_top(&self) -> usize {
        self.entries
            .last()
            .map_or(0, |e| e.lines.end.saturating_sub(1 + self.height))
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
        if let Some((path, top)) = &mut self.full_path {
            self.height = chrome::file_path(area, buf, path, top);
            self.footer = chrome::footer(area, buf, chrome::Footer::Path);
            return;
        }
        let wide = chrome::body_row(area, area.y).width >= 58;
        self.height = usize::from(area.height.saturating_sub(3));
        let mut line = 4;
        for row in &mut self.entries {
            if row.header.is_some() {
                line += 3;
            }
            let subtitle = row.detail.is_some() && (!wide || matches!(row.choice, Choice::Chat(_)));
            row.lines = line..line + 1 + usize::from(subtitle);
            line = row.lines.end;
        }
        self.top = self.top.min(self.max_top());
        if !self.wheel_scrolled && self.height > 0 {
            if let Some(row) = self.entries.get(self.selected) {
                let latest = row.lines.start - 1;
                let earliest = row.lines.end.saturating_sub(1 + self.height);
                if self.top != 0 || self.selected != 0 {
                    self.top = self
                        .top
                        .clamp(earliest.min(latest), latest)
                        .min(self.max_top());
                }
            }
        }
        chrome::header(
            area,
            buf,
            self.top,
            notice.map(|n| (n, !n.starts_with("Loading"))),
        );
        let waiting = !self.can_start && self.entries.len() == 1;
        if waiting {
            chrome::waiting(area, buf);
        } else {
            self.paint_rows(area, buf, pointer, focused, wide);
            chrome::arrows(area, buf, self.top > 0, self.top < self.max_top());
        }
        let choice = self.visible_choice();
        self.footer = chrome::footer(
            area,
            buf,
            chrome::Footer::Browse {
                open: choice.as_ref().map(|_| "open"),
                file_path: matches!(
                    choice,
                    Some(Choice::File(
                        FileTarget::Document { .. } | FileTarget::Directory(_)
                    ))
                ),
                archive: matches!(choice, Some(Choice::Chat(_))),
                movement: !waiting && self.entries.len() > 1,
            },
        );
    }

    fn paint_rows(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        pointer: Option<Position>,
        focused: bool,
        wide: bool,
    ) {
        let visible = 1 + self.top..1 + self.top + self.height;
        let rect = |line| chrome::body_row(area, area.y + (line - self.top) as u16);
        let width = usize::from(chrome::body_row(area, area.y).width.saturating_sub(1));
        for (i, entry) in self.entries.iter().enumerate() {
            if let Some(group) = &entry.header {
                let header = entry.lines.start - 2;
                if visible.contains(&header) {
                    Line::from(group.as_str()).dim().render(rect(header), buf);
                }
            }
            let start = entry.lines.start.max(visible.start);
            let end = entry.lines.end.min(visible.end);
            if start >= end {
                continue;
            }
            let hit = Rect {
                height: (end - start) as u16,
                ..rect(start)
            };
            let selected = i == self.selected;
            let style = if selected && focused {
                selection_style()
            } else if selected {
                Style::new().dim()
            } else if pointer.is_some_and(|p| hit.contains(p)) {
                Style::new().underlined()
            } else {
                Style::default()
            };
            if self.enabled(i) {
                self.hits.push((i, hit));
            }
            for y in start..end {
                let line = entry.line(y == entry.lines.start, wide, width, selected, style);
                let r = rect(y);
                buf.set_style(r, line.style);
                line.render(
                    Rect {
                        width: r.width.saturating_sub(1),
                        ..r
                    },
                    buf,
                );
            }
        }
    }
}
