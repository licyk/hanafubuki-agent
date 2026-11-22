// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! The chooser, document readers and chat tabs share a right pane. The
//! terminal's last painted rectangles are also its pointer targets.

mod chooser;
mod input;
mod strip;

use hanafubuki_agent::AgentSessionId;
use ratatui::{
    backend::Backend,
    buffer::Buffer,
    layout::{Position, Rect},
    style::Stylize,
    text::Line,
    widgets::{Clear, Widget},
};
use std::{
    io::{self, Write},
    path::PathBuf,
};

use super::{panes::split_title, App, PaneLayout, Parked, Workspace};
use crate::agent_bridge::AgentBridge;
use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::sidebar_files::{FileTarget, SidebarFiles};
use crate::text_formatting::strip_terminal_controls;
use crate::tui::Tui;
use chooser::{Choice, Chooser};
use strip::{TabState, TabStrip, TabTarget};

#[derive(Clone, Debug)]
enum Tab {
    Chat(AgentSessionId),
    Agent(AgentSessionId),
    Document(PathBuf),
}

impl PartialEq for Tab {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Chat(left), Self::Chat(right)) | (Self::Agent(left), Self::Agent(right)) => {
                left == right
            }
            (Self::Document(left), Self::Document(right)) => {
                hanafubuki_agent::PathKey::of(left) == hanafubuki_agent::PathKey::of(right)
            }
            _ => false,
        }
    }
}
impl Eq for Tab {}

#[derive(Default)]
pub(in crate::app) struct Sidebar {
    pub(in crate::app) files: SidebarFiles,
    owner: Option<AgentSessionId>,
    page: Option<Page>,
    docked: bool,
    focused: bool,
    tabs: Vec<(Tab, String)>,
    visited: Vec<Tab>,
    active: Option<Tab>,
    strip: Vec<(TabTarget<usize>, Rect)>,
    pointer: Option<Position>,
    press: Option<(input::PointerTarget, ratatui::crossterm::event::MouseButton)>,
    chooser: Chooser,
    pub(in crate::app) chooser_dirty: bool,
    close: Option<ConfirmClose>,
    pub(super) closing: std::collections::HashSet<AgentSessionId>,
    pub(super) notice: Option<String>,
    pressed_link: Option<(Position, FileTarget)>,
}

#[derive(Clone)]
enum Page {
    Chooser,
    Document(PathBuf),
}

struct ConfirmClose {
    id: AgentSessionId,
    archive: bool,
    message: String,
    confirm: bool,
    buttons: [Rect; 2],
}

impl Workspace {
    pub(in crate::app) fn sidebar_and_parked(&mut self) -> (&mut Sidebar, Option<&mut Parked>) {
        (
            &mut self.sidebar,
            self.second.as_mut().map(|second| &mut second.parked),
        )
    }

    pub(super) fn sidebar_layout(&self, layout: &mut PaneLayout) {
        let screen = layout.screen;
        let pane = if let Some((main, pane, divider)) = self.split_panes(screen) {
            layout.focused = main;
            layout.divider = Some(divider);
            pane
        } else if self.sidebar.owns_input() {
            layout.focused = Rect::default();
            screen
        } else {
            return;
        };
        let (strip, body) = split_title(pane);
        layout.strip = Some(strip);
        layout.panel = Some(body);
    }
}

impl Sidebar {
    pub(in crate::app) fn uses_fullscreen(&self) -> bool {
        self.docked || self.showing_panel()
    }

    pub(in crate::app) fn showing_panel(&self) -> bool {
        self.page.is_some()
    }

    pub(in crate::app) fn owns_input(&self) -> bool {
        self.showing_panel() && self.focused
    }

    pub(in crate::app) fn note_event(&self, event: &crate::app_event::AppEvent) {
        if matches!(self.page, Some(Page::Chooser))
            && matches!(event, crate::app_event::AppEvent::Agent(e) if Some(e.session_id) == self.owner && matches!(e.kind, hanafubuki_agent::AgentEventKind::HistoryCommitted { .. }))
        {
            self.files.note_commit();
        }
    }

    fn select(&mut self, tab: Option<Tab>) {
        if let Some(tab) = &tab {
            if self.visited.last() != Some(tab) {
                self.visited.retain(|seen| seen != tab);
                self.visited.push(tab.clone());
            }
        }
        self.active = tab;
    }

    pub(in crate::app) fn paint(
        &mut self,
        layout: &PaneLayout,
        buf: &mut Buffer,
    ) -> Option<crate::sidebar_files::ImagePlacement> {
        self.strip.clear();
        if let Some(area) = layout.strip {
            Clear.render(area, buf);
            let ids: Vec<_> = (0..self.tabs.len()).collect();
            let selected = self
                .tabs
                .iter()
                .position(|(tab, _)| Some(tab) == self.active.as_ref());
            let strip = TabStrip {
                ids: &ids,
                selected,
            };
            let chooser_state = if matches!(self.page, Some(Page::Chooser)) {
                TabState::Active
            } else {
                TabState::Absent
            };
            let selected_state = if self.focused {
                TabState::Active
            } else {
                TabState::Inactive
            };
            self.strip = strip.render(
                area,
                buf,
                chooser_state,
                selected_state,
                self.pointer,
                |index, rect, style, buf| {
                    let line =
                        Line::from(format!(" {}", strip_terminal_controls(&self.tabs[index].1)));
                    truncate_line_with_ellipsis_if_overflow(line, usize::from(rect.width))
                        .style(style)
                        .render(rect, buf);
                },
            );
        }
        let body = layout.panel?;
        Clear.render(body, buf);
        let image = match &self.page {
            Some(Page::Document(path)) => self.files.paint(path, body, buf),
            Some(Page::Chooser) => {
                self.chooser.paint(
                    body,
                    buf,
                    self.pointer,
                    self.focused,
                    self.notice.as_deref().or(self.files.notice()),
                );
                None
            }
            None => None,
        };
        if let Some(close) = self.close.as_mut() {
            Clear.render(body, buf);
            use ratatui::widgets::{Paragraph, Wrap};
            Paragraph::new(close.message.as_str())
                .wrap(Wrap { trim: false })
                .render(
                    Rect {
                        height: body.height.saturating_sub(2),
                        ..body
                    },
                    buf,
                );
            let buttons = Rect::new(body.x, body.bottom().saturating_sub(2), body.width, 1)
                .intersection(body);
            let width = buttons.width / 2;
            close.buttons = [
                Rect { width, ..buttons },
                Rect::new(
                    buttons.x + width,
                    buttons.y,
                    buttons.width - width,
                    buttons.height,
                ),
            ];
            for (i, label) in ["Cancel", "Confirm"].iter().enumerate() {
                let line = Line::from(*label);
                (if close.confirm == (i == 1) {
                    line.reversed()
                } else {
                    line
                })
                .render(close.buttons[i], buf);
            }
            return None;
        }
        image
    }
}

impl App {
    pub(in crate::app) fn prepare_sidebar_frame(&mut self) {
        if self.workspace.second.is_none() && !self.workspace.sidebar.showing_panel() {
            return;
        }
        let chats = self.sidebar_chats();
        let agent = self.active_sidebar_agent();
        let active = agent
            .map(Tab::Agent)
            .or_else(|| self.active_sidebar_chat().map(Tab::Chat));
        let focused = self.workspace.second_focused();
        let agents = &self.workspace.sidebar_agents;
        let sidebar = &mut self.workspace.sidebar;
        let mut available: Vec<_> = chats
            .iter()
            .map(|(id, label)| (Tab::Chat(*id), label.clone()))
            .collect();
        if let Some(id) = agent.or_else(|| {
            sidebar.tabs.iter().find_map(|(tab, _)| match tab {
                Tab::Agent(id) => Some(*id),
                _ => None,
            })
        }) {
            if agent.is_some() || agents.iter().any(|(a, _)| *a == id) {
                let title = agents
                    .iter()
                    .find(|(a, _)| *a == id)
                    .map_or_else(|| format!("Agent {id}"), |(_, s)| s.clone());
                available.push((Tab::Agent(id), title));
            }
        }
        for path in sidebar.files.open_paths() {
            let label = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
            available.push((Tab::Document(path), label));
        }
        sidebar
            .tabs
            .retain(|(tab, _)| available.iter().any(|(other, _)| other == tab));
        for (tab, label) in available {
            if let Some((_, name)) = sidebar.tabs.iter_mut().find(|(other, _)| other == &tab) {
                *name = label
            } else {
                sidebar.tabs.push((tab, label))
            }
        }
        sidebar
            .visited
            .retain(|tab| sidebar.tabs.iter().any(|(other, _)| other == tab));
        if sidebar.page.is_none() {
            sidebar.focused = focused;
            sidebar.select(active)
        }
        if matches!(sidebar.page, Some(Page::Chooser)) && std::mem::take(&mut sidebar.chooser_dirty)
        {
            sidebar.chooser.update(
                sidebar.owner,
                &chats,
                &self.workspace.saved_chats,
                agents,
                sidebar.files.entries(),
            );
        }
    }

    fn with_sidebar_files<R>(&mut self, f: impl FnOnce(&mut SidebarFiles, &AgentBridge) -> R) -> R {
        let mut files = std::mem::take(&mut self.workspace.sidebar.files);
        let result = f(&mut files, self.main_bridge());
        self.workspace.sidebar.files = files;
        result
    }

    pub(in crate::app) fn open_sidebar<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        self.workspace.sidebar.chooser_dirty = true;
        self.workspace.sidebar.notice = None;
        self.workspace.sidebar.chooser.dismiss_path();
        self.enter_panel(tui, Page::Chooser);
        self.request_sidebar_sessions();
        self.with_sidebar_files(|files, bridge| files.refresh(bridge));
    }

    fn enter_panel<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>, page: Page) {
        self.hide_side_chat(tui);
        self.enter_alt_screen(tui, true);
        let owner = self.main_session_id();
        let sidebar = &mut self.workspace.sidebar;
        sidebar.owner = owner;
        sidebar.docked = false;
        sidebar.focused = true;
        sidebar.select(match &page {
            Page::Document(path) => Some(Tab::Document(path.clone())),
            Page::Chooser => None,
        });
        sidebar.page = Some(page);
        self.schedule_frame(std::time::Instant::now());
    }

    fn enter_dock<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.enter_alt_screen(tui, true);
        self.workspace.sidebar.page = None;
        self.workspace.sidebar.close = None;
        self.workspace.sidebar.docked = true;
    }

    pub(super) fn leave_panel<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let open = self.workspace.sidebar.uses_fullscreen();
        self.workspace.sidebar.docked = false;
        self.workspace.sidebar.close = None;
        self.workspace.sidebar.strip.clear();
        self.workspace.sidebar.page = None;
        if open {
            self.leave_alt_screen(tui);
        }
    }

    pub(in crate::app) fn sync_sidebar_parent<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        let parent = self.main_session_id();
        if self.workspace.sidebar.owner != parent && !self.workspace.handling_parked() {
            self.leave_panel(tui);
            self.workspace.sidebar = Sidebar {
                owner: parent,
                ..Default::default()
            };
        }
    }

    fn choose_tab<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>, tab: Tab) {
        match tab {
            Tab::Document(path) => self.enter_panel(tui, Page::Document(path)),
            Tab::Chat(id) => {
                self.enter_dock(tui);
                self.select_side_chat(tui, id);
            }
            Tab::Agent(id) => {
                self.enter_dock(tui);
                self.dock_subagent(tui, id);
            }
        }
    }

    fn choose_file<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        target: FileTarget,
    ) {
        self.workspace.sidebar.chooser_dirty = true;
        if let Some(path) = self.with_sidebar_files(|files, bridge| files.open(target, bridge)) {
            self.enter_panel(tui, Page::Document(path));
        }
    }

    fn activate_choice<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        choice: Choice,
    ) {
        match choice {
            Choice::New => {
                self.enter_dock(tui);
                self.new_side_chat(tui, true);
            }
            Choice::Chat(id) => self.choose_tab(tui, Tab::Chat(id)),
            Choice::Agent(id) => self.choose_tab(tui, Tab::Agent(id)),
            Choice::File(target) => self.choose_file(tui, target),
        }
    }
}
