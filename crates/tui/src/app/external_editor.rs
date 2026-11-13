// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Ctrl+G (Codex's `App::launch_external_editor`): the composer's draft
//! edited in the person's editor, which has the terminal until it exits.
//! The footer says how to come back, for an editor in another window; the
//! terminal is released (`Tui::release`), the editor runs on the app's
//! thread, and the terminal is taken back and drawn again in full. The
//! Agent's events wait in the app's channel meanwhile. An editor that
//! cannot be found or started, fails, or leaves a file that cannot be read
//! leaves the draft as it was and says why in the transcript.

use std::io::{self, Write};
use std::time::Instant;

use ratatui::backend::Backend;

use super::App;
use crate::external_editor::run_editor;
use crate::tui::Tui;

impl App {
    pub(super) fn edit_in_external_editor<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        let command = match self.editor.command() {
            Ok(command) => command,
            Err(error) => return self.chat_widget.add_error_message(error.to_string()),
        };
        let seed = self.chat_widget.external_editor_seed();
        self.chat_widget.set_external_editor_open(true);
        let _ = self.draw(tui);
        let released = tui.release();
        let edited = run_editor(&command, &seed);
        let reclaimed = tui.reclaim(released);
        self.chat_widget.set_external_editor_open(false);
        match edited {
            Ok(text) => self.chat_widget.apply_external_edit(text),
            Err(error) => self.chat_widget.add_error_message(error.to_string()),
        }
        if let Err(error) = reclaimed {
            self.chat_widget.add_error_message(format!(
                "The terminal was not fully restored after the editor: {error}"
            ));
        }
        self.schedule_frame(Instant::now());
    }
}
