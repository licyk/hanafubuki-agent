// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use super::chooser::FooterAction;
use super::*;
use crate::keymap::{self, Action};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

#[derive(PartialEq, Eq)]
pub(super) enum PointerTarget {
    Chooser,
    Tab(Tab),
    Close(Tab),
    Row(Choice),
    Footer(FooterAction, Option<Choice>),
    Confirm(bool),
}

impl Sidebar {
    fn target_at(&mut self, pos: Position, button: MouseButton) -> Option<PointerTarget> {
        use PointerTarget as T;
        if let Some(close) = &self.close {
            return close
                .buttons
                .iter()
                .position(|r| r.contains(pos))
                .filter(|_| button == MouseButton::Left)
                .map(|i| T::Confirm(i == 1));
        }
        if let Some((target, _)) = self.strip.iter().find(|(_, r)| r.contains(pos)) {
            return match (button, target) {
                (MouseButton::Left, TabTarget::Add) => Some(T::Chooser),
                (MouseButton::Middle, TabTarget::Tab(i))
                | (MouseButton::Left | MouseButton::Middle, TabTarget::Close(i)) => {
                    self.tabs.get(*i).map(|(tab, _)| T::Close(tab.clone()))
                }
                (MouseButton::Left, TabTarget::Tab(i) | TabTarget::Overflow { tab: i, .. }) => {
                    self.tabs.get(*i).map(|(tab, _)| T::Tab(tab.clone()))
                }
                _ => None,
            };
        }
        if button != MouseButton::Left || !matches!(self.page, Some(Page::Chooser)) {
            return None;
        }
        if let Some(action) = self.chooser.footer_at(pos) {
            return Some(T::Footer(action, self.chooser.visible_choice()));
        }
        self.chooser.at(pos).map(T::Row)
    }
}

impl App {
    fn pointer_action<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        action: PointerTarget,
    ) {
        use PointerTarget as T;
        match action {
            T::Chooser => self.open_sidebar(tui),
            T::Tab(tab) => self.choose_tab(tui, tab),
            T::Close(tab) => self.close_tab(tui, tab, false),
            T::Row(choice) => self.activate_choice(tui, choice),
            T::Footer(FooterAction::Hide, _) => self.leave_panel(tui),
            T::Footer(action, _) => {
                let key = match action {
                    FooterAction::Open => KeyCode::Enter,
                    FooterAction::Archive => KeyCode::Delete,
                    FooterAction::Path => KeyCode::Char('p'),
                    FooterAction::Back | FooterAction::Hide => KeyCode::Esc,
                };
                self.panel_content_key(tui, KeyEvent::new(key, KeyModifiers::NONE));
            }
            T::Confirm(confirm) => {
                if let Some(close) = self.workspace.sidebar.close.as_mut() {
                    close.confirm = confirm;
                }
                self.confirm_close(tui)
            }
        }
    }
    fn close_tab<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        tab: Tab,
        archive: bool,
    ) {
        if let Tab::Chat(id) = tab {
            let losses = self.side_close_losses(id, archive);
            if !losses.is_empty() {
                let title = self
                    .workspace
                    .sidebar
                    .tabs
                    .iter()
                    .find_map(|(tab, title)| (tab == &Tab::Chat(id)).then_some(title))
                    .or_else(|| {
                        self.workspace
                            .saved_chats
                            .iter()
                            .find_map(|(chat, title)| (*chat == id).then_some(title))
                    })
                    .filter(|title| !title.trim().is_empty())
                    .map_or("Side chat", String::as_str);
                let title = truncate_line_with_ellipsis_if_overflow(
                    Line::from(strip_terminal_controls(title)),
                    24,
                );
                let verb = if archive { "Archive" } else { "Close" };
                let message = format!("This will {}. {verb} “{title}”?", losses.join(" and "));
                if !self.using_owned_view()
                    || self
                        .workspace
                        .layout
                        .as_ref()
                        .is_none_or(|layout| layout.strip.is_none())
                {
                    self.open_sidebar(tui);
                }
                self.workspace.sidebar.close = Some(ConfirmClose {
                    id,
                    archive,
                    message,
                    confirm: false,
                    buttons: Default::default(),
                });
                return;
            }
            self.workspace.sidebar.closing.insert(id);
            self.close_side_tab(tui, id, archive);
            return;
        } else if let Tab::Document(path) = &tab {
            self.workspace.sidebar.files.close(path)
        } else if matches!(&tab, Tab::Agent(id) if self.active_sidebar_agent() == Some(*id)) {
            self.hide_side_chat(tui);
        }
        self.after_close(tui, &tab);
    }

    fn after_close<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>, tab: &Tab) {
        let sidebar = &mut self.workspace.sidebar;
        sidebar.strip.clear();
        sidebar.tabs.retain(|(t, _)| t != tab);
        sidebar.visited.retain(|t| t != tab);
        if sidebar.active.as_ref().is_some_and(|t| t != tab) {
            return;
        }
        match sidebar.visited.last().cloned() {
            Some(tab) => self.choose_tab(tui, tab),
            None => self.open_sidebar(tui),
        }
    }

    pub(in crate::app::workspace) fn sidebar_tab_closed<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        id: AgentSessionId,
    ) {
        if self.workspace.sidebar.closing.remove(&id) {
            self.after_close(tui, &Tab::Chat(id));
        }
    }

    fn confirm_close<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let Some(close) = self.workspace.sidebar.close.take() else {
            return;
        };
        if close.confirm {
            self.workspace.sidebar.closing.insert(close.id);
            self.close_side_tab(tui, close.id, close.archive);
        }
    }

    fn cycle_tab<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        forward: bool,
    ) {
        let sidebar = &self.workspace.sidebar;
        let n = sidebar.tabs.len();
        if n == 0 {
            return;
        }
        let at = sidebar
            .tabs
            .iter()
            .position(|(tab, _)| Some(tab) == sidebar.active.as_ref());
        let next = if forward {
            at.map_or(0, |i| (i + 1) % n)
        } else {
            at.map_or(n - 1, |i| (i + n - 1) % n)
        };
        self.choose_tab(tui, sidebar.tabs[next].0.clone());
    }

    pub(in crate::app) fn sidebar_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        if key.kind == KeyEventKind::Release {
            return self.workspace.sidebar.owns_input();
        }
        if let Some(close) = self.workspace.sidebar.close.as_mut() {
            match key.code {
                KeyCode::Esc => self.workspace.sidebar.close = None,
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => close.confirm = !close.confirm,
                KeyCode::Enter => self.confirm_close(tui),
                _ => {}
            }
        } else if keymap::matches(Action::FocusSidebarTabs, &key) {
            self.open_sidebar(tui);
        } else if (self.workspace.sidebar.owns_input() || self.workspace.second_focused())
            && (keymap::matches(Action::SidebarUp, &key)
                || keymap::matches(Action::SidebarDown, &key))
        {
            self.cycle_tab(tui, keymap::matches(Action::SidebarDown, &key))
        } else if keymap::matches(Action::CloseSidebarTab, &key) {
            if let Some(tab) = self.workspace.sidebar.active.clone() {
                self.close_tab(tui, tab, false)
            } else {
                return false;
            }
        } else if self.workspace.second.is_none()
            && (keymap::matches(Action::FocusPaneRight, &key)
                || keymap::matches(Action::ToggleSideConversation, &key))
        {
            if !self.workspace.sidebar.showing_panel() {
                self.open_sidebar(tui)
            } else if keymap::matches(Action::ToggleSideConversation, &key) {
                self.workspace.sidebar.focused = !self.workspace.sidebar.focused
            } else {
                self.workspace.sidebar.focused = true
            }
        } else if self.workspace.sidebar.showing_panel()
            && keymap::matches(Action::FocusPaneLeft, &key)
        {
            self.workspace.sidebar.focused = false;
        } else if self.workspace.sidebar.showing_panel() && keymap::matches(Action::HidePane, &key)
        {
            self.leave_panel(tui);
        } else if self.workspace.sidebar.owns_input() {
            if let Some(action) = super::super::PaneAction::of(&key) {
                self.pane_action(tui, action);
            } else {
                self.panel_content_key(tui, key);
            }
        } else if self.active_sidebar_agent().is_some()
            && self.workspace.second_focused()
            && super::super::PaneAction::of(&key).is_none()
        {
            if self.transcript_view.has_active_interaction() {
                self.handle_owned_key(key);
            } else if keymap::matches(Action::SelectTranscript, &key) {
                self.transcript_view
                    .begin_select_mode(&self.transcript_cells);
            } else if key.modifiers.is_empty() {
                match key.code {
                    KeyCode::Esc => self.open_sidebar(tui),
                    KeyCode::Up => self.transcript_view.scroll(&self.transcript_cells, -1),
                    KeyCode::Down => self.transcript_view.scroll(&self.transcript_cells, 1),
                    KeyCode::Char('/') => self.transcript_view.begin_search(),
                    _ => {
                        self.handle_owned_key(key);
                    }
                }
            } else {
                self.handle_owned_key(key);
            }
        } else {
            return false;
        }
        self.schedule_frame(std::time::Instant::now());
        true
    }

    fn panel_content_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) {
        match self.workspace.sidebar.page.clone() {
            Some(Page::Document(path)) => {
                let captured = self.workspace.sidebar.files.captures_key(&path, key);
                match key.code {
                    KeyCode::Esc if !captured => self.open_sidebar(tui),
                    KeyCode::Left | KeyCode::Right
                        if !captured && key.modifiers == KeyModifiers::NONE =>
                    {
                        self.cycle_tab(tui, key.code == KeyCode::Right)
                    }
                    _ => {
                        if let Some(path) =
                            self.with_sidebar_files(|files, b| files.key(&path, key, b))
                        {
                            self.enter_panel(tui, Page::Document(path))
                        }
                    }
                }
            }
            Some(Page::Chooser) if self.workspace.sidebar.chooser.path_visible() => {
                let chooser = &mut self.workspace.sidebar.chooser;
                match key.code {
                    KeyCode::Esc | KeyCode::Char('p') | KeyCode::Enter => chooser.dismiss_path(),
                    KeyCode::Up | KeyCode::Char('k') => chooser.scroll(-1),
                    KeyCode::Down | KeyCode::Char('j') => chooser.scroll(1),
                    KeyCode::PageUp => chooser.scroll(-chooser.page()),
                    KeyCode::PageDown => chooser.scroll(chooser.page()),
                    _ => {}
                }
            }
            Some(Page::Chooser) => {
                let chooser = &mut self.workspace.sidebar.chooser;
                match key.code {
                    KeyCode::Esc => {
                        if let Some(recent) = self.workspace.sidebar.visited.last().cloned() {
                            self.choose_tab(tui, recent)
                        } else {
                            self.leave_panel(tui)
                        }
                    }
                    KeyCode::Char('p') => chooser.show_path(),
                    KeyCode::Enter => {
                        if let Some(choice) = chooser.visible_choice() {
                            self.activate_choice(tui, choice)
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => chooser.step(-1),
                    KeyCode::Down | KeyCode::Char('j') => chooser.step(1),
                    KeyCode::PageUp => chooser.step(-chooser.page()),
                    KeyCode::PageDown => chooser.step(chooser.page()),
                    KeyCode::Home => chooser.step(isize::MIN),
                    KeyCode::End => chooser.step(isize::MAX),
                    KeyCode::Left | KeyCode::Right => {
                        self.cycle_tab(tui, key.code == KeyCode::Right)
                    }
                    KeyCode::Delete => {
                        if let Some(Choice::Chat(id)) = chooser.visible_choice() {
                            self.close_tab(tui, Tab::Chat(id), true)
                        }
                    }
                    _ => {}
                }
            }
            None => {}
        }
    }

    pub(in crate::app) fn sidebar_mouse<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        mouse: MouseEvent,
    ) -> bool {
        let pos = Position::new(mouse.column, mouse.row);
        let click = mouse.kind == MouseEventKind::Down(MouseButton::Left);
        let wheel = match mouse.kind {
            MouseEventKind::ScrollUp => -3,
            MouseEventKind::ScrollDown => 3,
            _ => 0,
        };
        let sidebar = &mut self.workspace.sidebar;
        sidebar.pointer = Some(pos);
        let hit = self.workspace.layout.is_some() && !self.workspace.dragging;
        match mouse.kind {
            MouseEventKind::Down(button) => {
                sidebar.press = hit
                    .then(|| sidebar.target_at(pos, button))
                    .flatten()
                    .map(|action| (action, button));
            }
            MouseEventKind::Up(button) => {
                if let Some((pressed, pressed_button)) = sidebar.press.take() {
                    if hit
                        && button == pressed_button
                        && sidebar.target_at(pos, button).as_ref() == Some(&pressed)
                    {
                        self.pointer_action(tui, pressed);
                    }
                    self.schedule_frame(std::time::Instant::now());
                    return true;
                }
            }
            MouseEventKind::Drag(_) => sidebar.press = None,
            _ => {}
        }
        if sidebar.close.is_some() {
            // The confirmation owns the whole sidebar until a button is activated.
        } else if hit && sidebar.strip.iter().any(|(_, r)| r.contains(pos)) {
            if wheel != 0 {
                self.cycle_tab(tui, wheel > 0);
            }
        } else if self
            .workspace
            .layout
            .as_ref()
            .and_then(|l| l.panel)
            .is_some_and(|area| area.contains(pos))
            && !self.workspace.dragging
        {
            if click {
                sidebar.focused = true
            }
            match sidebar.page.clone() {
                Some(Page::Chooser) if wheel != 0 => sidebar.chooser.scroll(wheel),
                Some(Page::Document(path)) => {
                    if let Some(path) =
                        self.with_sidebar_files(|files, b| files.mouse(&path, mouse, b))
                    {
                        self.enter_panel(tui, Page::Document(path))
                    }
                }
                _ => {}
            }
        } else {
            if click
                && self
                    .workspace
                    .layout
                    .as_ref()
                    .is_some_and(|l| l.focused.contains(pos))
            {
                self.workspace.sidebar.focused = false;
                self.workspace.sidebar.pressed_link = self
                    .transcript_view
                    .file_link_at(mouse.column, mouse.row)
                    .map(|link| (pos, link));
            }
            if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
                self.workspace.sidebar.pressed_link = None
            }
            if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                if let Some((down, target)) = self.workspace.sidebar.pressed_link.take() {
                    if down == pos
                        && self
                            .transcript_view
                            .file_link_at(mouse.column, mouse.row)
                            .as_ref()
                            == Some(&target)
                    {
                        self.choose_file(tui, target);
                        return true;
                    }
                }
            }
            return false;
        }
        self.schedule_frame(std::time::Instant::now());
        true
    }

    pub(in crate::app) fn sidebar_paste(&mut self, text: &str) -> bool {
        if self.workspace.sidebar.close.is_some() {
            return true;
        }
        if self.active_sidebar_agent().is_some() && self.workspace.second_focused() {
            if self.transcript_view.paste_search(text) {
                self.schedule_frame(std::time::Instant::now());
            }
            return true;
        }
        if !self.workspace.sidebar.owns_input() {
            return false;
        }
        if let Some(Page::Document(path)) = &self.workspace.sidebar.page {
            self.workspace.sidebar.files.paste(path, text);
        }
        self.schedule_frame(std::time::Instant::now());
        true
    }
}
