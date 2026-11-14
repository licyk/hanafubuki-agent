// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The bottom-pane footer renders transient hints under the composer.
//!
//! The footer is pure rendering: it formats `FooterProps` into `Line`s without mutating any state.
//! It intentionally does not decide *which* footer content should be shown; that is owned by the
//! `ChatComposer` (which selects a `FooterMode`) and by the `ChatWidget` (which decides when quit
//! or interrupt is allowed).
//!
//! The "press again to quit" hint is time-based rather than event-based; the owning widgets
//! schedule a redraw so it can expire even if the UI is otherwise idle.
//!
//! After a first Esc on an empty composer, the footer says a second one edits a previous message
//! (`FooterMode::EscHint`). The status line (`/statusline`) takes the row of
//! the shortcuts hint while the composer is empty, and while a draft is
//! written with no turn running; a draft written during a turn shows the key
//! that queues it instead. `?` shows the shortcuts in its place
//! ([`shortcut_overlay`]). A newer release's notice takes the right of the
//! one-row modes' row, in full, shortened or not at all, so it never cuts
//! the row's own content and adds no row. Codex's footer also shows the context window left, the collaboration
//! mode and its width-based collapse rules; those are not ported.

mod shortcut_overlay;

use crate::key_hint;
use crate::key_hint::KeyBinding;
use crate::keymap::{self, Action};
use crate::render::line_utils::prefix_lines;
use crate::ui_consts::FOOTER_INDENT_COLS;
use crate::update_notice::{notice_line, TuiUpdateNotice};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

/// The rendering inputs for the footer area under the composer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FooterProps<'a> {
    pub(crate) mode: FooterMode,
    pub(crate) use_shift_enter_hint: bool,
    /// Which key the user must press again to quit.
    ///
    /// This is rendered when `mode` is `FooterMode::QuitShortcutReminder`.
    pub(crate) quit_shortcut_key: KeyBinding,
    /// The status line, when this mode shows it.
    pub(crate) status_line: Option<&'a Line<'static>>,
    /// A newer release, shown on the right where the row has room.
    pub(crate) update_notice: Option<&'a TuiUpdateNotice>,
    /// A turn runs: a draft shows the key that queues it, and the
    /// shortcuts say that key queues.
    pub(crate) task_running: bool,
    /// Left opens the command center: the empty composer's row says so.
    pub(crate) agents_hint: bool,
    /// A second conversation (a side conversation, a subagent shown): the
    /// mode, the effort and the earlier messages are the main one's.
    pub(crate) second_conversation: bool,
    /// The draft starts with `!`: it is a shell command.
    pub(crate) shell_command: bool,
    /// Under WSL, whose terminal pastes text with Ctrl+V: the shortcuts
    /// show Ctrl+Alt+V for pasting an image.
    pub(crate) wsl: bool,
}

/// Selects which footer content is rendered.
///
/// The current mode is owned by `ChatComposer`, which may override it based on transient state
/// (for example, showing `QuitShortcutReminder` only while its timer is active).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FooterMode {
    /// Transient "press again to quit" reminder (Ctrl+C/Ctrl+D).
    QuitShortcutReminder,
    /// Multi-line shortcut overlay shown after pressing `?`.
    ShortcutOverlay,
    /// Base single-line footer when the composer is empty.
    ComposerEmpty,
    /// Base single-line footer when the composer contains a draft.
    ComposerHasDraft,
    /// After a first Esc on an empty composer: a second one browses the
    /// prompts to edit an earlier message.
    EscHint,
}

pub(crate) fn toggle_shortcut_mode(
    current: FooterMode,
    ctrl_c_hint: bool,
    is_empty: bool,
) -> FooterMode {
    if ctrl_c_hint && matches!(current, FooterMode::QuitShortcutReminder) {
        return current;
    }

    let base_mode = if is_empty {
        FooterMode::ComposerEmpty
    } else {
        FooterMode::ComposerHasDraft
    };

    match current {
        FooterMode::ShortcutOverlay | FooterMode::QuitShortcutReminder => base_mode,
        _ => FooterMode::ShortcutOverlay,
    }
}

pub(crate) fn reset_mode_after_activity(current: FooterMode) -> FooterMode {
    match current {
        FooterMode::ShortcutOverlay
        | FooterMode::QuitShortcutReminder
        | FooterMode::ComposerHasDraft
        | FooterMode::EscHint => FooterMode::ComposerEmpty,
        other => other,
    }
}

pub(crate) fn footer_height(props: FooterProps<'_>, width: u16) -> u16 {
    let rows = match props.mode {
        FooterMode::ShortcutOverlay => {
            shortcut_overlay::lines(&props, width.saturating_sub(FOOTER_INDENT_COLS as u16)).len()
        }
        _ => footer_lines(props).len(),
    };
    rows as u16
}

/// Render footer content from `FooterProps`, indented like the composer.
pub(crate) fn render_footer(area: Rect, buf: &mut Buffer, props: FooterProps<'_>) {
    if props.mode == FooterMode::ShortcutOverlay {
        let indent = (FOOTER_INDENT_COLS as u16).min(area.width);
        let area = Rect {
            x: area.x + indent,
            width: area.width - indent,
            ..area
        };
        return shortcut_overlay::render(&props, area, buf);
    }
    let lines = footer_lines(props);
    let content = FOOTER_INDENT_COLS + lines.first().map_or(0, Line::width);
    Paragraph::new(prefix_lines(
        lines,
        " ".repeat(FOOTER_INDENT_COLS).into(),
        " ".repeat(FOOTER_INDENT_COLS).into(),
    ))
    .render(area, buf);
    if let Some(notice) = props.update_notice {
        render_update_notice(area, buf, notice, content);
    }
}

/// The columns between the row's content and the update notice.
const NOTICE_GAP: usize = 3;
/// The columns left free after the update notice.
const NOTICE_RIGHT_MARGIN: usize = 2;

/// The notice at the right of the row whose own content takes `content`
/// columns, in the room that leaves.
fn render_update_notice(area: Rect, buf: &mut Buffer, notice: &TuiUpdateNotice, content: usize) {
    let room = usize::from(area.width).saturating_sub(content + NOTICE_GAP + NOTICE_RIGHT_MARGIN);
    let Some(line) = notice_line(notice, room).filter(|_| area.height > 0) else {
        return;
    };
    // It fits in `room`, which is less than the area's width.
    let width = line.width() as u16;
    let x = area.right() - NOTICE_RIGHT_MARGIN as u16 - width;
    buf.set_line(x, area.y, &line, width);
}

/// Renders one line chosen by the composer (the history search's) in
/// place of the footer, indented as the footer is.
pub(crate) fn render_footer_line(area: Rect, buf: &mut Buffer, line: Line<'static>) {
    Paragraph::new(prefix_lines(
        vec![line],
        " ".repeat(FOOTER_INDENT_COLS).into(),
        " ".repeat(FOOTER_INDENT_COLS).into(),
    ))
    .render(area, buf);
}

/// The canonical FooterMode-to-text mapping.
fn footer_lines(props: FooterProps<'_>) -> Vec<Line<'static>> {
    match props.mode {
        FooterMode::QuitShortcutReminder => {
            vec![quit_shortcut_reminder_line(props.quit_shortcut_key)]
        }
        FooterMode::ComposerEmpty => vec![composer_empty_line(props)],
        FooterMode::ShortcutOverlay => shortcut_overlay::lines(&props, u16::MAX),
        FooterMode::ComposerHasDraft if props.shell_command => vec![Line::from(vec![
            "!".red().bold(),
            " shell command · runs in the session's sandbox".dim(),
        ])],
        FooterMode::ComposerHasDraft => vec![match keymap::hint(Action::Queue) {
            Some(key) if props.task_running => {
                Line::from(vec![key.into(), " to queue message".dim()])
            }
            _ => props.status_line.cloned().unwrap_or_default(),
        }],
        FooterMode::EscHint => vec![Line::from(vec![
            keymap::hint(Action::EditPrevious)
                .unwrap_or(key_hint::plain(KeyCode::Esc))
                .into(),
            " again to edit a previous message".into(),
        ])
        .dim()],
    }
}

/// The status line, else `? for shortcuts` (the key that shows them);
/// with `← for agents` after the status line, or before the shortcuts
/// (Codex's `passive_footer_suffix` and `left_side_line`).
fn composer_empty_line(props: FooterProps<'_>) -> Line<'static> {
    let agents = [key_hint::plain(KeyCode::Left).into(), " for agents".dim()];
    let shortcuts = keymap::hint(Action::ShowShortcuts)
        .map(|key| vec![key.into(), " for shortcuts".dim()])
        .unwrap_or_default();
    match (props.status_line, props.agents_hint) {
        (Some(status), false) => status.clone(),
        (Some(status), true) => {
            let mut line = status.clone();
            line.push_span(" · ".dim());
            line.spans.extend(agents);
            line
        }
        (None, false) => Line::from(shortcuts),
        (None, true) => {
            let mut spans = agents.to_vec();
            if !shortcuts.is_empty() {
                spans.push(" · ".dim());
                spans.extend(shortcuts);
            }
            Line::from(spans)
        }
    }
}

fn quit_shortcut_reminder_line(key: KeyBinding) -> Line<'static> {
    Line::from(vec![key.into(), " again to quit".into()]).dim()
}

#[cfg(test)]
mod tests;
