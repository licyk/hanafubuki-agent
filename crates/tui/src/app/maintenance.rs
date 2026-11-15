// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! As with the external editor, a host operation temporarily owns the terminal.
use super::App;
use crate::maintenance::TuiMaintenanceAction;
use crate::tui::Tui;
use ratatui::backend::Backend;
use std::io::{self, Write};

impl App {
    pub(super) fn run_maintenance<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        action: TuiMaintenanceAction,
    ) {
        let Some(maintenance) = self.maintenance.clone() else {
            self.chat_widget
                .add_error_message("Maintenance is unavailable in this host".into());
            return;
        };
        let released = tui.release();
        let result = (maintenance.0)(action);
        if let Err(error) = &result {
            eprintln!("{error}");
        }
        println!("\nPress Enter to return to the Agent.");
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
        let reclaimed = tui.reclaim(released);
        if let Err(error) = result {
            self.chat_widget.add_error_message(error);
        }
        if let Err(error) = reclaimed {
            self.chat_widget.add_error_message(error.to_string());
        }
        self.schedule_frame(std::time::Instant::now());
    }
}
