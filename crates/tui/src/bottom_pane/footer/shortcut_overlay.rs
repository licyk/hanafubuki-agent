// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The shortcuts `?` shows (Codex's `bottom_pane/shortcut_overlay.rs`):
//! Compose, Session and Transcript, in three, two or one column by width
//! (`shortcut_help`), then `/keymap customize`. Each key is the keymap's
//! in force, so a remapped action shows its new key and an unbound one, or
//! one with only a chord, is left out; `/`, `@`, `!`, Ctrl+C and PgUp/PgDn
//! are fixed. Paste image shows Ctrl+Alt+V under WSL, as Codex does. Codex's
//! voice key is not ported. A cut list ends with a note naming `/keymap`.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::FooterProps;
use crate::key_hint::{self, KeyBinding};
use crate::keymap::{self, Action, Binding};
use crate::shortcut_help::{group_lines, label, Group, Shortcut};
use crate::style::accent_color;
use crate::wrapping::word_wrap_lines;

pub(super) fn lines(props: &FooterProps<'_>, width: u16) -> Vec<Line<'static>> {
    let mut compose = Group {
        title: "Compose",
        entries: vec![
            Shortcut::new(key_hint::plain(KeyCode::Char('/')), "Commands"),
            Shortcut::new(key_hint::plain(KeyCode::Char('@')), "Mention files"),
            Shortcut::new(key_hint::plain(KeyCode::Char('!')), "Shell command"),
        ],
    };
    compose.push(newline_key(props), "New line");
    compose.push(paste_image_key(props), "Paste image");
    compose.push(keymap::hint(Action::ExternalEditor), "External editor");
    compose.push(keymap::hint(Action::HistorySearch), "Search history");
    if !props.second_conversation {
        if let Some(key) = keymap::hint(Action::EditPrevious) {
            let key = label(key);
            compose.entries.push(Shortcut {
                key: format!("{key} {key}"),
                action: "Edit last message",
            });
        }
    }

    let mut session = Group {
        title: "Session",
        entries: Vec::new(),
    };
    let queue = if props.task_running {
        "Queue message"
    } else {
        "Send message"
    };
    session.push(keymap::hint(Action::Queue), queue);
    if !props.second_conversation {
        session.push(keymap::hint(Action::CycleMode), "Change mode");
        session.push(keymap::hint(Action::ReasoningDown), "Less reasoning");
        session.push(keymap::hint(Action::ReasoningUp), "More reasoning");
    }
    if props.agents_hint {
        session.push(
            Some(key_hint::plain(KeyCode::Left)),
            "Agents (empty prompt)",
        );
    }
    session.push(keymap::hint(Action::OpenAgents), "Agents");
    if !keymap::current().bindings(Action::OpenTasks).is_empty() {
        session.entries.push(Shortcut {
            key: keymap::current().label(Action::OpenTasks),
            action: "Background tasks",
        });
    }
    if let Some(keys) = pair(Action::PreviousAgent, Action::NextAgent) {
        session.entries.push(Shortcut {
            key: keys,
            action: "Subagents (empty prompt)",
        });
    }
    session.push(keymap::hint(Action::FocusActivity), "Inspect activity");
    let ctrl_c = if props.task_running {
        "Interrupt"
    } else {
        "Quit"
    };
    session.push(Some(key_hint::ctrl(KeyCode::Char('c'))), ctrl_c);

    let mut transcript = Group {
        title: "Transcript (open first)",
        entries: Vec::new(),
    };
    transcript.push(keymap::hint(Action::OpenTranscript), "Open transcript");
    transcript.push(keymap::hint(Action::FindTranscript), "Find text");
    transcript.entries.push(Shortcut {
        key: "pgup / pgdn".to_string(),
        action: "Scroll",
    });
    transcript.push(keymap::hint(Action::StartSelection), "Start selection");
    if let Some(keys) = pair(Action::TranscriptTop, Action::TranscriptBottom) {
        transcript.entries.push(Shortcut {
            key: keys,
            action: "Top / latest",
        });
    }
    // Both jumps' alternatives: the terminal may be on another system.
    let shifted = |c| KeyBinding::new(KeyCode::Char(c), KeyModifiers::ALT | KeyModifiers::SHIFT);
    let alt_shift = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT | KeyModifiers::SHIFT);
    if keymap::matches(Action::TranscriptTop, &alt_shift(','))
        && keymap::matches(Action::TranscriptBottom, &alt_shift('.'))
    {
        transcript.entries.push(Shortcut {
            key: format!("{} / {}", label(shifted(',')), label(shifted('.'))),
            action: "Top / latest",
        });
    }

    let mut result = vec![Line::from("Keyboard shortcuts").bold(), Line::default()];
    result.extend(group_lines([compose, session, transcript], width));
    let width = usize::from(width.max(1));
    result.push(Line::default());
    result.extend(footer_lines(width));
    word_wrap_lines(&result, width)
}

/// Draws the shortcuts in `area`; cut to its height, the list keeps its
/// footer and says where the rest is.
pub(super) fn render(props: &FooterProps<'_>, area: Rect, buf: &mut Buffer) {
    let mut lines = lines(props, area.width);
    let height = usize::from(area.height);
    if lines.len() > height && height > 0 {
        let mut footer = footer_lines(usize::from(area.width.max(1)));
        footer.truncate(height);
        let body_height = height.saturating_sub(footer.len());
        lines.truncate(body_height.saturating_sub(1));
        if body_height > 0 {
            lines.push(Line::from("… more: /keymap").dim());
        }
        lines.extend(footer);
    }
    Paragraph::new(lines).render(area, buf);
}

fn footer_lines(width: usize) -> Vec<Line<'static>> {
    word_wrap_lines(
        [Line::from(vec![
            "/keymap".fg(accent_color()),
            " customize".dim(),
        ])],
        width,
    )
}

/// The key that inserts a new line: Shift+Enter where the terminal reports
/// it, else the first other key.
fn newline_key(props: &FooterProps<'_>) -> Option<KeyBinding> {
    let shift_enter = key_hint::shift(KeyCode::Enter);
    let keys: Vec<KeyBinding> = keymap::current()
        .bindings(Action::InsertNewline)
        .iter()
        .filter_map(|binding| match binding {
            Binding::Key(key) => Some(key.hint()),
            _ => None,
        })
        .collect();
    if props.use_shift_enter_hint && keys.contains(&shift_enter) {
        Some(shift_enter)
    } else {
        keys.into_iter().find(|key| *key != shift_enter)
    }
}

/// The key that pastes an image: Ctrl+Alt+V under WSL where it is bound,
/// else the first.
fn paste_image_key(props: &FooterProps<'_>) -> Option<KeyBinding> {
    let wsl_key = KeyBinding::new(
        KeyCode::Char('v'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );
    let keys: Vec<KeyBinding> = keymap::current()
        .bindings(Action::PasteImage)
        .iter()
        .filter_map(|binding| match binding {
            Binding::Key(key) => Some(key.hint()),
            _ => None,
        })
        .collect();
    if props.wsl && keys.contains(&wsl_key) {
        Some(wsl_key)
    } else {
        keys.into_iter().next()
    }
}

/// Two actions' keys as `first / second`, when both have one.
fn pair(first: Action, second: Action) -> Option<String> {
    let first = keymap::hint(first)?;
    let second = keymap::hint(second)?;
    Some(format!("{} / {}", label(first), label(second)))
}
