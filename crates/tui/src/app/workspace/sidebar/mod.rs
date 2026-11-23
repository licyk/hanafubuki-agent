// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! The chooser, document readers and chat tabs share a right pane. The
//! terminal's last painted rectangles are also its pointer targets.

mod chooser;
mod help;
mod input;
mod prompts;
mod strip;

use hanafubuki_agent::AgentSessionId;
use ratatui::{
    backend::Backend,
    buffer::Buffer,
    layout::{Position, Rect},
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
    prompts: std::collections::HashMap<AgentSessionId, Option<String>>,
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
        self.close.is_some() || (self.showing_panel() && self.focused)
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
        let strip = layout.strip?;
        let pane = Rect {
            x: strip.x,
            width: strip.width,
            ..layout.screen
        };
        if let Some(close) = self.close.as_mut() {
            close.buttons = close.paint(pane, buf);
            return None;
        }
        let image = match (&self.page, layout.panel) {
            (Some(Page::Document(path)), Some(body)) => {
                self.files.paint(path, body, buf, None, true)
            }
            (Some(Page::Chooser), Some(_)) => {
                self.chooser.paint(
                    pane,
                    buf,
                    self.pointer,
                    self.focused,
                    self.notice.as_deref().or(self.files.notice()),
                );
                None
            }
            _ => None,
        };
        Clear.render(strip, buf);
        let ids: Vec<_> = (0..self.tabs.len()).collect();
        let selected = self
            .tabs
            .iter()
            .position(|(tab, _)| Some(tab) == self.active.as_ref());
        let selector = TabStrip {
            ids: &ids,
            selected,
        };
        let showing_chooser = matches!(self.page, Some(Page::Chooser));
        let focus = if self.focused {
            TabState::Active
        } else {
            TabState::Inactive
        };
        self.strip = selector.render(
            strip,
            buf,
            if showing_chooser {
                focus
            } else {
                TabState::Absent
            },
            if showing_chooser {
                TabState::Absent
            } else {
                focus
            },
            self.pointer,
            |index, rect, style, buf| {
                let width = usize::from(rect.width.saturating_sub(1));
                let (tab, label) = &self.tabs[index];
                let label = strip_terminal_controls(label);
                let mut line = match tab {
                    Tab::Chat(_) => {
                        truncate_line_with_ellipsis_if_overflow(Line::from(label), width)
                    }
                    Tab::Agent(_) => Line::from(crate::line_truncation::middle_label(
                        &label,
                        width.saturating_sub(1),
                    )),
                    Tab::Document(path) => {
                        Line::from(strip::document_title(path, &self.tabs, width))
                    }
                };
                line.spans.insert(0, " ".into());
                line.style(style).render(rect, buf);
            },
        );
        image
    }

    pub(in crate::app) fn document_cursor(&self) -> Option<Position> {
        match &self.page {
            Some(Page::Document(path)) if self.focused && self.close.is_none() => {
                self.files.cursor(path)
            }
            _ => None,
        }
    }
}

impl ConfirmClose {
    fn paint(&self, area: Rect, buf: &mut Buffer) -> [Rect; 2] {
        Clear.render(area, buf);
        if area.width < 3 || area.height < 3 {
            return Default::default();
        }
        let body = area.inner(ratatui::layout::Margin::new(1, 0));
        let lines = textwrap::wrap(&self.message, usize::from(body.width));
        let offset = lines.len() + 2;
        let accept = if self.archive { " Archive " } else { " Close " };
        if body.width < 10 + accept.len() as u16 || offset >= usize::from(area.height) {
            Line::from("Resize to review · Esc to cancel").render(body, buf);
            return Default::default();
        }
        for (line, row) in lines.into_iter().zip(body.rows().skip(1)) {
            Line::from(line).render(row, buf);
        }
        let y = area.y + offset as u16;
        let buttons = [
            Rect::new(body.x, y, 8, 1),
            Rect::new(body.x + 10, y, accept.len() as u16, 1),
        ];
        for (index, (label, button)) in [" Cancel ", accept].into_iter().zip(buttons).enumerate() {
            let line = Line::from(label);
            if self.confirm == (index == 1) {
                line.style(crate::style::selection_style())
            } else {
                line
            }
            .render(button, buf);
        }
        if let Some(footer) = body.rows().nth(offset + 2) {
            use crate::footer_hint::{shortcut, HintLine};
            HintLine::new(
                [
                    shortcut("←/→", "choose"),
                    shortcut("Enter", "confirm"),
                    shortcut("Esc", "cancel"),
                ],
                footer.width,
            )
            .line
            .render(footer, buf);
        }
        buttons
    }
}

fn chat_label(title: &str, ordinal: usize, multiple: bool, prefix: &str) -> String {
    match title.trim().lines().next() {
        Some(name) => name.trim().to_owned(),
        None if multiple => format!("{prefix} {}", ordinal + 1),
        None => "Side chat".into(),
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
        let multiple = chats.len() > 1
            || self
                .workspace
                .saved_chats
                .iter()
                .any(|(id, _)| !chats.iter().any(|(open, _)| open == id));
        let mut available: Vec<_> = chats
            .iter()
            .enumerate()
            .map(|(ordinal, (id, label))| {
                (Tab::Chat(*id), chat_label(label, ordinal, multiple, "Side"))
            })
            .collect();
        if let Some(id) = agent.or_else(|| {
            sidebar.tabs.iter().find_map(|(tab, _)| match tab {
                Tab::Agent(id) => Some(*id),
                _ => None,
            })
        }) {
            if agent.is_some() || agents.iter().any(|a| a.session_id == id) {
                let title = agents
                    .iter()
                    .find(|a| a.session_id == id)
                    .map_or_else(|| format!("Agent {id}"), |a| a.title.clone());
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
                &sidebar.files,
                &sidebar.prompts,
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
        self.workspace.sidebar.prompts.clear();
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
            Choice::Main => {
                self.leave_child(tui);
                self.leave_panel(tui);
            }
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
