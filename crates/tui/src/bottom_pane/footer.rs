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
//! written with no turn running. Codex's footer also shows the context window left, the collaboration
//! mode and its width-based collapse rules, and shortcuts for shell commands, images, the external
//! editor and backtracking; none of those are ported.
use crate::key_hint;
use crate::key_hint::KeyBinding;
use crate::render::line_utils::prefix_lines;
use crate::ui_consts::FOOTER_INDENT_COLS;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
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
    /// What Ctrl-O does, while the model's thinking is live.
    pub(crate) thinking_toggle: Option<ThinkingToggle>,
    /// The status line, when this mode shows it.
    pub(crate) status_line: Option<&'a Line<'static>>,
}

/// What the thinking toggle (Ctrl-O) does to the live thinking cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ThinkingToggle {
    Expand,
    Collapse,
}

impl ThinkingToggle {
    fn hint(self) -> Line<'static> {
        let label = match self {
            ThinkingToggle::Expand => " to show thinking",
            ThinkingToggle::Collapse => " to hide thinking",
        };
        Line::from(vec![
            key_hint::ctrl(KeyCode::Char('o')).into(),
            label.into(),
        ])
    }
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

pub(crate) fn footer_height(props: FooterProps<'_>) -> u16 {
    footer_lines(props).len() as u16
}

/// Render footer content from `FooterProps`, indented like the composer.
pub(crate) fn render_footer(area: Rect, buf: &mut Buffer, props: FooterProps<'_>) {
    Paragraph::new(prefix_lines(
        footer_lines(props),
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
        FooterMode::ComposerEmpty => {
            let mut line = props.status_line.cloned().unwrap_or_else(|| {
                Line::from(vec![
                    key_hint::plain(KeyCode::Char('?')).into(),
                    " for shortcuts".dim(),
                ])
            });
            if let Some(toggle) = props.thinking_toggle {
                line.push_span("   ".dim());
                line.extend(toggle.hint().dim().spans);
            }
            vec![line]
        }
        FooterMode::ShortcutOverlay => shortcut_overlay_lines(props),
        FooterMode::ComposerHasDraft => vec![props.status_line.cloned().unwrap_or_default()],
        FooterMode::EscHint => vec![Line::from(vec![
            key_hint::plain(KeyCode::Esc).into(),
            " again to edit a previous message".into(),
        ])
        .dim()],
    }
}

fn quit_shortcut_reminder_line(key: KeyBinding) -> Line<'static> {
    Line::from(vec![key.into(), " again to quit".into()]).dim()
}

fn shortcut_overlay_lines(props: FooterProps<'_>) -> Vec<Line<'static>> {
    let newline = if props.use_shift_enter_hint {
        key_hint::shift(KeyCode::Enter)
    } else {
        key_hint::ctrl(KeyCode::Char('j'))
    };
    let entry = |key: KeyBinding, label: &'static str| Line::from(vec![key.into(), label.into()]);
    let mut entries = vec![
        entry(key_hint::plain(KeyCode::Char('/')), " for commands"),
        entry(newline, " for newline"),
        entry(key_hint::plain(KeyCode::Char('@')), " for file paths"),
        entry(key_hint::plain(KeyCode::Esc), " to interrupt"),
        entry(key_hint::plain(KeyCode::Up), " for previous message"),
        entry(key_hint::ctrl(KeyCode::Char('c')), " to exit"),
        entry(key_hint::ctrl(KeyCode::Char('t')), " to view transcript"),
    ];
    entries.extend(props.thinking_toggle.map(ThinkingToggle::hint));
    build_columns(entries)
}

fn build_columns(entries: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return Vec::new();
    }

    const COLUMNS: usize = 2;
    const COLUMN_PADDING: [usize; COLUMNS] = [4, 4];
    const COLUMN_GAP: usize = 4;

    let rows = entries.len().div_ceil(COLUMNS);
    let target_len = rows * COLUMNS;
    let mut entries = entries;
    if entries.len() < target_len {
        entries.extend(std::iter::repeat_n(
            Line::from(""),
            target_len - entries.len(),
        ));
    }

    let mut column_widths = [0usize; COLUMNS];

    for (idx, entry) in entries.iter().enumerate() {
        let column = idx % COLUMNS;
        column_widths[column] = column_widths[column].max(entry.width());
    }

    for (idx, width) in column_widths.iter_mut().enumerate() {
        *width += COLUMN_PADDING[idx];
    }

    entries
        .chunks(COLUMNS)
        .map(|chunk| {
            let mut line = Line::from("");
            for (col, entry) in chunk.iter().enumerate() {
                line.extend(entry.spans.clone());
                if col < COLUMNS - 1 {
                    let target_width = column_widths[col];
                    let padding = target_width.saturating_sub(entry.width()) + COLUMN_GAP;
                    line.push_span(Span::from(" ".repeat(padding)));
                }
            }
            line.dim()
        })
        .collect()
}

#[cfg(test)]
mod tests;
