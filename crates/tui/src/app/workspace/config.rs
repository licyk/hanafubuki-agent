// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/config`'s full-screen center in the app: open over the whole screen,
//! hand keys to its model, and carry its actions back to the command loop.

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::crossterm::event::KeyEvent;

use super::App;
use crate::agent_bridge::ConfigState;
use crate::app_event::{AppCommand, AppEvent};
use crate::config_center::{ConfigAction, ConfigCenter, ConfigChange, ConfigSection};
use crate::tui::Tui;

impl App {
    pub(super) fn open_config_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        section: ConfigSection,
        direct: bool,
    ) {
        if self.workspace.config.is_some() {
            return;
        }
        let state = self.bridge.config_state();
        self.enter_alt_screen(tui, false);
        let mut center = ConfigCenter::new(state);
        center.open(section, direct);
        self.workspace.config = Some(center);
    }

    /// The center's key, while it is open: `false` when it is not.
    pub(crate) fn handle_config_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        let Some(center) = self.workspace.config.as_mut() else {
            return false;
        };
        let action = center.handle_key(key);
        if let Some(action) = action {
            self.run_config_action(tui, action);
        }
        self.schedule_frame(std::time::Instant::now());
        true
    }

    /// Draws the center over the screen.
    pub(crate) fn draw_config_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<bool> {
        let Some(center) = self.workspace.config.as_mut() else {
            return Ok(false);
        };
        tui.draw(u16::MAX, |frame| {
            let area = frame.area();
            center.render(area, frame.buffer_mut());
        })?;
        Ok(true)
    }

    fn run_config_action<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        action: ConfigAction,
    ) {
        match action {
            ConfigAction::Close => self.close_config_center(tui),
            ConfigAction::Replay(command) => {
                self.close_config_center(tui);
                self.app_event_tx
                    .send(AppEvent::Command(AppCommand::Replay(command)));
            }
            ConfigAction::Save(change) => {
                let change = *change;
                let (saved, shadowed) = self.write_config_change(change);
                if let Some(center) = self.workspace.config.as_mut() {
                    center.save_finished(saved, shadowed);
                }
            }
        }
    }

    fn write_config_change(&mut self, change: ConfigChange) -> (Result<ConfigState, String>, bool) {
        let result = match change.clone() {
            ConfigChange::Agent(patch) => self.bridge.write_agent_settings(patch),
            ConfigChange::Standalone(patch) => self.bridge.write_standalone_settings(patch),
            ConfigChange::Tui(patch) => match self.bridge.write_tui_preferences(patch) {
                Ok(preferences) => {
                    self.apply_preferences(preferences);
                    self.bridge.config_state()
                }
                Err(error) => Err(error),
            },
        };
        let shadowed = result
            .as_ref()
            .ok()
            .is_some_and(|state| change.is_shadowed_by(state));
        (result, shadowed)
    }

    fn close_config_center<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.workspace.config = None;
        self.leave_alt_screen(tui);
    }
}
