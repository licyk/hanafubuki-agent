//! `/config`'s full-screen center in the app: open over the whole screen,
//! hand keys to its model, and carry its actions back to the command loop.

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::crossterm::event::KeyEvent;

use super::App;
use crate::app_event::{AppCommand, AppEvent};
use crate::config_center::{ConfigAction, ConfigCenter, ConfigSection};
use crate::tui::Tui;

impl App {
    pub(super) fn open_config_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        section: ConfigSection,
    ) {
        if self.workspace.config.is_some() {
            return;
        }
        let state = self.bridge.config_state();
        self.enter_alt_screen(tui, false);
        let mut center = ConfigCenter::new(state);
        center.select(section);
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
        self.close_config_center(tui);
        if let ConfigAction::Replay(command) = action {
            self.app_event_tx
                .send(AppEvent::Command(AppCommand::Replay(command)));
        }
    }

    fn close_config_center<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.workspace.config = None;
        self.leave_alt_screen(tui);
    }
}
