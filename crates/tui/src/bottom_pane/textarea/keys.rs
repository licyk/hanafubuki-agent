// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The editing keys, from the keymap's `editor` context (Codex's
//! `input_with_keymap`): emacs-style motions and kills, word moves, new
//! lines. Terminals that report control chords as C0 characters match the
//! same keys (the keymap reads them as Ctrl and the letter). A plain
//! character types itself.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyModifiers;

use super::TextArea;
use crate::key_hint::is_altgr;
use crate::keymap::{self, Action};

impl TextArea {
    pub fn input(&mut self, event: KeyEvent) {
        let keymap = keymap::current();
        let pressed = |action| keymap.matches(action, &event);
        if pressed(Action::InsertNewline) {
            self.insert_str("\n");
        } else if pressed(Action::DeleteBackwardWord) {
            self.delete_backward_word();
        } else if let (KeyCode::Char(c), true) = (event.code, is_altgr(event.modifiers)) {
            // Windows AltGr generates ALT|CONTROL: a character, unless a
            // binding above took it.
            self.insert_str(&c.to_string());
        } else if pressed(Action::DeleteBackward) {
            self.delete_backward(1);
        } else if pressed(Action::DeleteForwardWord) {
            self.delete_forward_word();
        } else if pressed(Action::DeleteForward) {
            self.delete_forward(1);
        } else if pressed(Action::KillLineStart) {
            self.kill_to_beginning_of_line();
        } else if pressed(Action::KillLineEnd) {
            self.kill_to_end_of_line();
        } else if pressed(Action::Yank) {
            self.yank();
        } else if pressed(Action::MoveWordLeft) {
            self.set_cursor(self.beginning_of_previous_word());
        } else if pressed(Action::MoveWordRight) {
            self.set_cursor(self.end_of_next_word());
        } else if pressed(Action::MoveLeft) {
            self.move_cursor_left();
        } else if pressed(Action::MoveRight) {
            self.move_cursor_right();
        } else if pressed(Action::MoveUp) {
            self.move_cursor_up();
        } else if pressed(Action::MoveDown) {
            self.move_cursor_down();
        } else if pressed(Action::MoveLineStart) {
            // Ctrl-A at a line's start goes on to the line above (Emacs).
            self.move_cursor_to_beginning_of_line(is_ctrl(event, 'a'));
        } else if pressed(Action::MoveLineEnd) {
            self.move_cursor_to_end_of_line(is_ctrl(event, 'e'));
        } else if let KeyEvent {
            code: KeyCode::Char(c),
            // Not with Alt: terminals map Option/Meta combos to Alt+<char>
            // for word motions.
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        } = event
        {
            if !c.is_ascii_control() {
                self.insert_str(&c.to_string());
            }
        }
    }
}

fn is_ctrl(event: KeyEvent, letter: char) -> bool {
    event.code == KeyCode::Char(letter) && event.modifiers == KeyModifiers::CONTROL
}
