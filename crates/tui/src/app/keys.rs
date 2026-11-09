// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Where a key goes: the resume picker and the command center take them
//! all while open; then a chord's first key waits for its second
//! (`keymap::chords`), and a completed chord goes on as its action's token;
//! then the panes' keys (`workspace`), editing an earlier message, an
//! overlay, the owned transcript's keys and the app's own
//! actions (Ctrl+T, select mode), and the chat last. A view that captures
//! keys (`/keymap`'s) gets every key as it is.

use std::io::{self, Write};
use std::time::Instant;

use ratatui::backend::Backend;
use ratatui::crossterm::event::KeyEvent;

use super::App;
use crate::app_event::{AgentCommand, AppCommand, AppEvent, Op};
use crate::keymap::{self, Action, ChordStep, Context};
use crate::pager_overlay::{Overlay, TranscriptOverlay};
use crate::resume_picker::SessionSelection;
use crate::tui::Tui;

/// The contexts read beside the composer.
const MAIN: [Context; 5] = [
    Context::Global,
    Context::Chat,
    Context::Composer,
    Context::Editor,
    Context::Transcript,
];
const PAGER: [Context; 2] = [Context::Global, Context::Pager];

impl App {
    /// `false` when the app should exit.
    pub(super) fn handle_key_event<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        if self.resume_picker.is_some() {
            return self.handle_picker_key(tui, key);
        }
        if self.handle_agents_key(tui, key) {
            return true;
        }
        let contexts: &[Context] = if self.chat_widget.captures_keys() {
            &[]
        } else if self.overlay.is_some() {
            &PAGER
        } else {
            &MAIN
        };
        let key = match self
            .chords
            .feed(&keymap::current(), &key, contexts, Instant::now())
        {
            ChordStep::Pass => key,
            ChordStep::Complete(token) => token,
            ChordStep::Wait | ChordStep::Cancelled => {
                self.schedule_frame(Instant::now());
                return true;
            }
        };
        if contexts.is_empty() {
            self.chat_widget.handle_key_event(key);
            return true;
        }
        self.dispatch_key(tui, key)
    }

    fn handle_picker_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        let Some(picker) = self.resume_picker.as_mut() else {
            return true;
        };
        let selection = picker.handle_key(key);
        self.schedule_frame(Instant::now());
        let Some(selection) = selection else {
            self.load_picker_pages();
            return true;
        };
        self.resume_picker = None;
        // The session left is replaced, header and all: what it wrote
        // while the picker was open is not shown.
        if matches!(selection, SessionSelection::Resume(_)) {
            self.deferred_history_lines.clear();
        }
        self.leave_alt_screen(tui);
        match selection {
            SessionSelection::StartFresh => {}
            SessionSelection::Resume(session_id) => self
                .app_event_tx
                .send(AppEvent::Op(Op::Command(AgentCommand::Resume(session_id)))),
            SessionSelection::Exit => return false,
        }
        true
    }

    fn dispatch_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        if self.handle_workspace_key(tui, key) {
            return true;
        }
        // An earlier message of a second conversation is not edited.
        if !self.workspace.second_focused() && self.handle_backtrack_key(tui, key) {
            return true;
        }
        if let Some(overlay) = self.overlay.as_mut() {
            let action = match overlay {
                Overlay::Transcript(transcript) => transcript.handle_key(key),
                Overlay::Static(pager) => {
                    pager.handle_key_event(key, tui.terminal.viewport_area);
                    None
                }
            };
            if overlay.is_done() {
                self.overlay = None;
                self.reset_backtrack();
                self.leave_alt_screen(tui);
            } else if let Some(action) = action {
                self.apply_view_action(action);
            }
            self.schedule_frame(Instant::now());
            return true;
        }
        if self.mode.is_owned() {
            if self.handle_owned_key(key) {
                self.schedule_frame(Instant::now());
                return true;
            }
            if keymap::matches(Action::SelectTranscript, &key)
                && self.chat_widget.no_modal_or_popup_active()
            {
                self.run_app_command(tui, AppCommand::SelectTranscript);
                return true;
            }
        } else if is_transcript_key(key) {
            let overlay = TranscriptOverlay::new(self.transcript_cells.clone(), self.render_mode);
            self.open_overlay(tui, Overlay::Transcript(overlay));
            return true;
        }
        self.chat_widget.handle_key_event(key);
        true
    }
}

/// The transcript overlay's key in terminal mode, the detailed
/// presentation's in owned mode.
pub(super) fn is_transcript_key(key: KeyEvent) -> bool {
    keymap::matches(Action::OpenTranscript, &key)
}
