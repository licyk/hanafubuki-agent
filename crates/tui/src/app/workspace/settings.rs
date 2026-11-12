// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/config`'s settings center in the app: open over the whole screen (the
//! alternate screen in Scrollback) with the pointer, its keys, clicks and
//! pastes handed to it, its calls run off the loop, and a page that is a
//! command's views drawn from the command's view in the bottom pane, which
//! keeps its replies and its next steps.

use std::io::{self, Write};
use std::time::Instant;

use ratatui::backend::Backend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use super::WorkspaceEvent;
use crate::agent_bridge::CatalogReply;
use crate::app::App;
use crate::app_event::AppEvent;
use crate::settings_center::{CenterRequest, SettingsCenter};
use crate::tui::Tui;

impl App {
    pub(super) fn open_settings_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        target: Option<String>,
    ) {
        if self.workspace.settings.is_some() {
            return;
        }
        self.enter_alt_screen(tui, true);
        self.workspace.settings = Some(SettingsCenter::new(target));
        self.run_settings_requests(tui);
    }

    /// The center's key, while it is open: `false` when it is not. A page
    /// that is a command's views gets it as the command's view would.
    pub(crate) fn handle_settings_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        let Some(center) = self.workspace.settings.as_mut() else {
            return false;
        };
        match center.hosted_command() {
            Some(_) => {
                // Ctrl-C closes the view as Esc does, never the interface.
                let key = if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')
                {
                    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
                } else {
                    key
                };
                self.chat_widget.handle_key_event(key);
                self.app_event_tx
                    .untagged()
                    .send(AppEvent::Workspace(WorkspaceEvent::SettingsPageCheck));
            }
            None => center.handle_key(key),
        }
        self.run_settings_requests(tui);
        self.schedule_frame(Instant::now());
        true
    }

    /// The pointer, while the center is open: `false` when it is not.
    pub(crate) fn handle_settings_mouse<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        mouse: MouseEvent,
    ) -> bool {
        let Some(center) = self.workspace.settings.as_mut() else {
            return false;
        };
        if center.hosted_command().is_none() {
            center.handle_mouse(mouse);
            self.run_settings_requests(tui);
            self.schedule_frame(Instant::now());
        }
        true
    }

    /// Pasted text, while the center is open: `false` when it is not.
    pub(crate) fn handle_settings_paste(&mut self, text: &str) -> bool {
        let Some(center) = self.workspace.settings.as_mut() else {
            return false;
        };
        match center.hosted_command() {
            Some(_) => self.chat_widget.handle_paste(text.to_string()),
            None => center.handle_paste(text),
        }
        self.schedule_frame(Instant::now());
        true
    }

    /// Draws the center over the screen, and the command's view that is
    /// its page.
    pub(crate) fn draw_settings_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<bool> {
        let Some(center) = self.workspace.settings.as_mut() else {
            return Ok(false);
        };
        let chat = &self.chat_widget;
        tui.draw(u16::MAX, |frame| {
            let area = frame.area();
            let hosted = center.render(area, frame.buffer_mut());
            let mut cursor = center.cursor();
            if let (Some(body), Some(command)) = (hosted, center.hosted_command()) {
                if let Some(view) = chat.command_view(command) {
                    let height = view.desired_height(body.width).min(body.height);
                    let body = ratatui::layout::Rect { height, ..body };
                    view.render(body, frame.buffer_mut());
                    cursor = view.cursor_pos(body);
                }
            }
            if let Some(position) = cursor {
                frame.set_cursor_position(position);
            }
        })?;
        Ok(true)
    }

    pub(super) fn settings_reply<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        reply: CatalogReply,
    ) {
        let preferences = reply.preferences.clone();
        if let Some(center) = self.workspace.settings.as_mut() {
            center.on_reply(reply);
            self.run_settings_requests(tui);
        }
        if let Some(preferences) = preferences {
            self.adopt_tui_preferences(preferences);
        }
    }

    /// The page that is a command's views closes with its last view.
    pub(super) fn check_settings_page(&mut self) {
        let Some(center) = self.workspace.settings.as_mut() else {
            return;
        };
        if let Some(command) = center.hosted_command() {
            if !self.chat_widget.has_command_view(command) {
                center.hosted_closed();
                self.run_settings_requests_quietly();
            }
        }
    }

    /// Carries out what the center asked.
    fn run_settings_requests<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let Some(center) = self.workspace.settings.as_mut() else {
            return;
        };
        for request in center.take_requests() {
            match request {
                CenterRequest::Close => {
                    self.workspace.settings = None;
                    self.leave_alt_screen(tui);
                    return;
                }
                request => self.run_settings_request(request),
            }
        }
    }

    /// [`Self::run_settings_requests`] where the center cannot ask to
    /// close.
    fn run_settings_requests_quietly(&mut self) {
        let requests = self
            .workspace
            .settings
            .as_mut()
            .map(SettingsCenter::take_requests)
            .unwrap_or_default();
        for request in requests {
            self.run_settings_request(request);
        }
    }

    fn run_settings_request(&mut self, request: CenterRequest) {
        match request {
            CenterRequest::Call(id, call) => self.bridge.catalog_call(id, call),
            CenterRequest::OpenCommand(command) => {
                self.chat_widget.dispatch_command(command, String::new());
                self.app_event_tx
                    .untagged()
                    .send(AppEvent::Workspace(WorkspaceEvent::SettingsPageCheck));
            }
            CenterRequest::Close => {}
        }
    }
}
