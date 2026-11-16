// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Task shortcuts, adapted from `agent_center/hints.rs`.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::{input::SESSION_ACTIONS, AgentsCenter};
use crate::key_hint;
use crate::keymap::{self, Action};
use crate::shortcut_help::{self, Group, Shortcut};

impl AgentsCenter {
    pub(super) fn render_help(&mut self, area: Rect, buf: &mut Buffer) {
        let lines = help_lines(area.width);
        self.view_rows = Some(usize::from(area.height));
        self.help_scroll = self
            .help_scroll
            .min(lines.len().saturating_sub(usize::from(area.height)));
        Paragraph::new(lines)
            .scroll((self.help_scroll as u16, 0))
            .render(area, buf);
    }

    pub(super) fn help_key(&mut self, key: KeyEvent) {
        let page = self.view_rows.unwrap_or(10).max(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.help_scroll = self.help_scroll.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.help_scroll = self.help_scroll.saturating_add(1)
            }
            KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(page),
            KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(page),
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => self.help_scroll = usize::MAX,
            _ => {}
        }
    }
}

fn help_lines(width: u16) -> Vec<Line<'static>> {
    let mut navigate = Group {
        title: "Navigate",
        entries: Vec::new(),
    };
    for (code, label) in [
        (KeyCode::Up, "Up"),
        (KeyCode::Down, "Down"),
        (KeyCode::Enter, "Open"),
        (KeyCode::PageUp, "Page up"),
        (KeyCode::PageDown, "Page down"),
    ] {
        if !claimed(code) {
            navigate
                .entries
                .push(Shortcut::new(key_hint::plain(code), label));
        }
    }
    navigate.entries.push(Shortcut {
        key: "ctrl+c".into(),
        action: "Quit",
    });
    let mut tasks = Group {
        title: "Tasks",
        entries: Vec::new(),
    };
    for (action, label) in [
        (Action::AgentsNewTask, "New"),
        (Action::AgentsNewWorktree, "New worktree"),
        (Action::AgentsResume, "Resume"),
        (Action::AgentsFork, "Fork"),
        (Action::AgentsRename, "Rename"),
        (Action::AgentsStop, "Stop"),
        (Action::AgentsArchive, "Archive"),
        (Action::AgentsHide, "Hide"),
        (Action::AgentsDelete, "Delete"),
        (Action::AgentsPin, "Pin/unpin"),
    ] {
        push_action(&mut tasks, action, label);
    }
    let mut view = Group {
        title: "View",
        entries: Vec::new(),
    };
    let filters = [(KeyCode::Tab, "tab"), (KeyCode::BackTab, "shift+tab")]
        .into_iter()
        .filter(|(code, _)| !claimed(*code))
        .map(|(_, label)| label)
        .collect::<Vec<_>>()
        .join("/");
    if !filters.is_empty() {
        view.entries.push(Shortcut {
            key: filters,
            action: "Filter",
        });
    }
    for (action, label) in [
        (Action::AgentsSearch, "Search"),
        (Action::AgentsToggleGrouping, "Group"),
        (Action::AgentsChildren, "Children"),
    ] {
        push_action(&mut view, action, label);
    }
    let mut lines = vec!["Task shortcuts".bold().into(), Line::default()];
    lines.extend(shortcut_help::group_lines([navigate, tasks, view], width));
    crate::wrapping::word_wrap_lines(lines, usize::from(width.max(1)))
}

fn push_action(group: &mut Group, action: Action, label: &'static str) {
    let keymap = keymap::current();
    let Some(binding) = keymap.bindings(action).first() else {
        return;
    };
    let key = match binding {
        crate::keymap::Binding::Key(key) => shortcut_help::label(key.hint()).replace(" + ", "+"),
        _ => binding.spec(),
    };
    group.entries.push(Shortcut { key, action: label });
}

fn claimed(code: KeyCode) -> bool {
    let key = KeyEvent::from(code);
    SESSION_ACTIONS
        .into_iter()
        .any(|action| keymap::matches(action, &key))
}
