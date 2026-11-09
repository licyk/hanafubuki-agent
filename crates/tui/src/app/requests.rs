// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the slash commands ask of the app itself: a copy to the clipboard
//! (`/copy`, `/export`), select mode in the owned transcript, the cleared
//! screen of `/clear`, a message `/init` submits, and `/resume TEXT`'s
//! lookup (Codex's `ClearUi`, `CopySelection` and
//! `ResumeSessionByIdOrName` events).

use std::io::{self, Write};
use std::time::Instant;

use ratatui::backend::Backend;

use super::{new_view, App};
use crate::app_event::{AgentCommand, AppCommand, AppEvent, Op};
use crate::history_cell;
use crate::resume_picker::{find_session, ResumePicker};
use crate::transcript_view::TranscriptHistoryState;
use crate::tui::Tui;

impl App {
    pub(super) fn run_app_command<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        command: AppCommand,
    ) {
        match command {
            AppCommand::Copy(text) => self.copy_text(&text),
            AppCommand::SelectTranscript => {
                if !self
                    .transcript_view
                    .begin_select_mode(&self.transcript_cells)
                {
                    self.add_info("Nothing to select yet");
                }
                self.schedule_frame(Instant::now());
            }
            AppCommand::ClearScreen => self.clear_screen(tui),
            AppCommand::Submit(text) => self.chat_widget.submit_user_message(text),
            AppCommand::ResumeNamed(text) => self.resume_named(tui, text),
        }
    }

    /// Copies `text` and says so in the transcript.
    fn copy_text(&mut self, text: &str) {
        let cell = match self.clipboard.copy(text) {
            Ok(status) => history_cell::new_info_event(status.message(text.chars().count()), None),
            Err(error) => history_cell::new_error_event(format!("Copy failed: {error}")),
        };
        self.app_event_tx
            .send(AppEvent::InsertHistoryCell(Box::new(cell)));
    }

    /// Clears what the session showed: the terminal's scrollback and
    /// screen, or the owned transcript.
    fn clear_screen<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.transcript_cells.clear();
        self.deferred_history_lines.clear();
        self.has_emitted_history_lines = false;
        if self.mode.is_owned() {
            self.transcript_view = new_view(self.render_mode, TranscriptHistoryState::Complete);
        } else if let Err(error) = tui.clear_scrollback() {
            self.add_info(&format!("The screen was not cleared: {error}"));
        }
        self.schedule_frame(Instant::now());
    }

    /// Resumes the session `text` names, or opens the picker searching for
    /// it.
    fn resume_named<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        text: String,
    ) {
        match find_session(&text, |request| self.bridge.list_sessions(request)) {
            Ok(Some(session_id)) => self
                .app_event_tx
                .send(AppEvent::Op(Op::Command(AgentCommand::Resume(session_id)))),
            Ok(None) => {
                let picker = ResumePicker::new(false, self.bridge.session_id()).with_query(text);
                self.show_resume_picker(tui, picker);
            }
            Err(error) => {
                let cell = history_cell::new_error_event(format!("/resume: {error}"));
                self.app_event_tx
                    .send(AppEvent::InsertHistoryCell(Box::new(cell)));
            }
        }
    }

    fn add_info(&self, message: &str) {
        let cell = history_cell::new_info_event(message.to_string(), None);
        self.app_event_tx
            .send(AppEvent::InsertHistoryCell(Box::new(cell)));
    }
}
