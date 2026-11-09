// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Captures a key, a chord or a leader chord inside `/keymap` (Codex's
//! `keymap_setup/capture.rs`). The view takes every key as it is pressed,
//! before any binding; Esc cancels. A key that cannot be bound, or that
//! conflicts, is refused in the view with the reason, and another can be
//! tried.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::save;
use crate::app_event::AppEventSender;
use crate::bottom_pane::{render_menu_surface, BottomPaneView, CancellationEvent};
use crate::keymap::{self, parse_keys, Action, Binding, KeyStroke};
use crate::render::renderable::Renderable;

/// What is captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Capture {
    Key,
    Chord,
    Leader,
}

pub(super) struct KeymapCapture {
    action: Action,
    capture: Capture,
    /// The new binding is added to the action's keys, not put in their
    /// place.
    add: bool,
    first: Option<KeyStroke>,
    error: Option<String>,
    app_event_tx: AppEventSender,
    complete: bool,
}

impl KeymapCapture {
    pub(super) fn new(action: Action, capture: Capture, add: bool, tx: AppEventSender) -> Self {
        Self {
            action,
            capture,
            add,
            first: None,
            error: None,
            app_event_tx: tx,
            complete: false,
        }
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let keymap = keymap::current();
        let instructions = match (self.capture, self.first) {
            (Capture::Key, _) => "Press the new key now. Esc cancels.".to_string(),
            (Capture::Chord, None) => {
                "Press the first key, then the second. Esc cancels.".to_string()
            }
            (Capture::Chord, Some(first)) => {
                format!(
                    "First key: {}. Press the second key. Esc cancels.",
                    first.spec()
                )
            }
            (Capture::Leader, _) => format!(
                "Press the key that follows the leader ({}). Esc cancels.",
                keymap.label(Action::Leader)
            ),
        };
        let mut lines = vec![
            Line::from(
                format!(
                    "{} {}",
                    if self.add {
                        "Add a key to"
                    } else {
                        "New key for"
                    },
                    self.action.path()
                )
                .bold(),
            ),
            Line::from(vec!["Now: ".dim(), keymap.label(self.action).cyan()]),
            Line::from(instructions.dim()),
        ];
        lines.extend(self.error.clone().map(|error| Line::from(error.red())));
        lines
    }

    /// The binding pressed: the action's keys with it saved, or why not.
    fn captured(&mut self, binding: Binding) {
        let keymap = keymap::current();
        let mut bindings = if self.add {
            keymap.bindings(self.action).to_vec()
        } else {
            Vec::new()
        };
        if !bindings.contains(&binding) {
            bindings.push(binding);
        }
        let result = parse_keys(self.action, &[binding.spec()])
            .and_then(|_| save(&self.app_event_tx, self.action, Some(bindings)));
        match result {
            Ok(()) => self.complete = true,
            Err(error) => self.error = Some(error),
        }
    }
}

impl BottomPaneView for KeymapCapture {
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            self.complete = true;
            return;
        }
        let Some(stroke) = KeyStroke::of(&key) else {
            self.error = Some("That key cannot be bound.".to_string());
            return;
        };
        self.error = None;
        match (self.capture, self.first.take()) {
            (Capture::Key, _) => self.captured(Binding::Key(stroke)),
            (Capture::Leader, _) => self.captured(Binding::Leader(stroke)),
            (Capture::Chord, None) => self.first = Some(stroke),
            (Capture::Chord, Some(first)) => self.captured(Binding::Chord(first, stroke)),
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }

    fn captures_keys(&self) -> bool {
        true
    }
}

impl Renderable for KeymapCapture {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let content = render_menu_surface(area, buf);
        Paragraph::new(self.lines())
            .wrap(Wrap { trim: false })
            .render(content, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let width = width.saturating_sub(4).max(1);
        let rows: u16 = self
            .lines()
            .iter()
            .map(|line| (line.width() as u16).div_ceil(width).max(1))
            .sum();
        rows + 2
    }
}
