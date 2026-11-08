// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The editing keys: emacs-style motions and kills, word moves, and the
//! fallbacks for terminals that report control chords as C0 characters.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyModifiers;

use super::TextArea;
use crate::key_hint::is_altgr;

impl TextArea {
    pub fn input(&mut self, event: KeyEvent) {
        match event {
            // Some terminals (or configurations) send Control key chords as
            // C0 control characters without reporting the CONTROL modifier.
            // Handle common fallbacks for Ctrl-B/F/P/N here so they don't get
            // inserted as literal control bytes.
            KeyEvent { code: KeyCode::Char('\u{0002}'), modifiers: KeyModifiers::NONE, .. } /* ^B */ => {
                self.move_cursor_left();
            }
            KeyEvent { code: KeyCode::Char('\u{0006}'), modifiers: KeyModifiers::NONE, .. } /* ^F */ => {
                self.move_cursor_right();
            }
            KeyEvent { code: KeyCode::Char('\u{0010}'), modifiers: KeyModifiers::NONE, .. } /* ^P */ => {
                self.move_cursor_up();
            }
            KeyEvent { code: KeyCode::Char('\u{000e}'), modifiers: KeyModifiers::NONE, .. } /* ^N */ => {
                self.move_cursor_down();
            }
            KeyEvent {
                code: KeyCode::Char(c),
                // Insert plain characters (and Shift-modified). Do NOT insert when ALT is held,
                // because many terminals map Option/Meta combos to ALT+<char> (e.g. ESC f/ESC b)
                // for word navigation. Those are handled explicitly below.
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => self.insert_str(&c.to_string()),
            KeyEvent {
                code: KeyCode::Char('j' | 'm'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self.insert_str("\n"),
            KeyEvent {
                code: KeyCode::Char('h'),
                modifiers,
                ..
            } if modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.delete_backward_word()
            },
            // Windows AltGr generates ALT|CONTROL; treat as a plain character input unless
            // we match a specific Control+Alt binding above.
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if is_altgr(modifiers) => self.insert_str(&c.to_string()),
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::ALT,
                ..
            } => self.delete_backward_word(),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.delete_backward(1),
            KeyEvent {
                code: KeyCode::Delete,
                modifiers: KeyModifiers::ALT,
                ..
            }  => self.delete_forward_word(),
            KeyEvent {
                code: KeyCode::Delete,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.delete_forward(1),

            KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.delete_backward_word();
            }
            // Meta-b -> move to beginning of previous word
            // Meta-f -> move to end of next word
            // Many terminals map Option (macOS) to Alt. Some send Alt|Shift, so match contains(ALT).
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::ALT,
                ..
            } => {
                self.set_cursor(self.beginning_of_previous_word());
            }
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::ALT,
                ..
            } => {
                self.set_cursor(self.end_of_next_word());
            }
            KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.kill_to_beginning_of_line();
            }
            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.kill_to_end_of_line();
            }
            KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.yank();
            }

            // Cursor movement
            KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_cursor_left();
            }
            KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_cursor_right();
            }
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_cursor_left();
            }
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_cursor_right();
            }
            KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_cursor_up();
            }
            KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_cursor_down();
            }
            // Some terminals send Alt+Arrow for word-wise movement:
            // Option/Left -> Alt+Left (previous word start)
            // Option/Right -> Alt+Right (next word end)
            KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::ALT,
                ..
            }
            | KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.set_cursor(self.beginning_of_previous_word());
            }
            KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::ALT,
                ..
            }
            | KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.set_cursor(self.end_of_next_word());
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.move_cursor_up();
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                self.move_cursor_down();
            }
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => {
                self.move_cursor_to_beginning_of_line(false);
            }
            KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_cursor_to_beginning_of_line(true);
            }

            KeyEvent {
                code: KeyCode::End, ..
            } => {
                self.move_cursor_to_end_of_line(false);
            }
            KeyEvent {
                code: KeyCode::Char('e'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_cursor_to_end_of_line(true);
            }
            _ => {}
        }
    }
}
