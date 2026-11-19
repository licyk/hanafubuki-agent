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
//! (`FooterMode::EscHint`). The status line (`/statusline`) precedes the
//! shortcut and transcript-navigation hints. A draft written during a turn
//! shows the key that queues it; `?` expands the shortcuts below the status
//! line ([`shortcut_overlay`]). Codex's footer also shows the context window left, the collaboration
//! mode and its width-based collapse rules; those are not ported.

mod goal;
mod shortcut_overlay;

pub(crate) use goal::GoalStatus;

use crate::appearance::{SideCount, SideItem};
use crate::key_hint;
use crate::key_hint::KeyBinding;
use crate::keymap::{self, Action};
use crate::render::line_utils::prefix_lines;
use crate::ui_consts::FOOTER_INDENT_COLS;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

/// What the footer shows beside the hints: the counts on the second row's
/// right, the status line's items placed on its left, and what happens when
/// a row does not fit.
#[derive(Clone, Debug, Default)]
pub(crate) struct FooterSides {
    pub(crate) counts: Vec<SideCount>,
    pub(crate) second_row: Option<Line<'static>>,
    pub(crate) overflow: Overflow,
}

/// What the footer does when a line does not fit beside what shares it:
/// `wrap-left` keeps the right side on its row and moves the left side
/// below it, `wrap-all` moves both, `truncate` cuts the left side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Overflow {
    WrapLeft,
    #[default]
    WrapAll,
    Truncate,
}

/// The rendering inputs for the footer area under the composer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FooterProps<'a> {
    pub(crate) mode: FooterMode,
    pub(crate) use_shift_enter_hint: bool,
    /// Which key the user must press again to quit.
    ///
    /// This is rendered when `mode` is `FooterMode::QuitShortcutReminder`.
    pub(crate) quit_shortcut_key: KeyBinding,
    /// Transcript navigation replaces the ordinary shortcut hints.
    pub(crate) navigation: Option<&'a [Line<'static>]>,
    /// Session information above the hints.
    pub(crate) status_line: Option<&'a Line<'static>>,
    pub(crate) goal: Option<&'a GoalStatus>,
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
    /// The counts on the second row's right: terminals, agents, warnings.
    pub(crate) counts: &'a [SideCount],
    /// The status line's items whose place is the second row's left.
    pub(crate) second_row: Option<&'a Line<'static>>,
    /// Where each count was drawn, for a click: the composer's own, shared
    /// with the render that fills it.
    pub(crate) hits: &'a std::cell::RefCell<Vec<(Rect, SideItem)>>,
    /// When a row does not fit beside the counts.
    pub(crate) overflow: Overflow,
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
                + usize::from(props.status_line.is_some())
        }
        _ => wrapped_footer(props, width).len(),
    };
    rows as u16
}

/// Render footer content from `FooterProps`, indented like the composer.
pub(crate) fn render_footer(area: Rect, buf: &mut Buffer, props: FooterProps<'_>) {
    if area.is_empty() {
        return;
    }
    if props.mode == FooterMode::ShortcutOverlay {
        let indent = (FOOTER_INDENT_COLS as u16).min(area.width);
        let mut area = Rect {
            x: area.x + indent,
            width: area.width - indent,
            ..area
        };
        if let Some(status) = props.status_line {
            buf.set_line(area.x, area.y, status, area.width);
            area.y = area.y.saturating_add(1);
            area.height = area.height.saturating_sub(1);
        }
        return shortcut_overlay::render(&props, area, buf, goal_line(props));
    }
    let lines = wrapped_footer(props, area.width);
    record_count_hits(area, props, &lines);
    Paragraph::new(prefix_lines(
        lines,
        " ".repeat(FOOTER_INDENT_COLS).into(),
        " ".repeat(FOOTER_INDENT_COLS).into(),
    ))
    .render(area, buf);
}

/// Where each count landed, so a click on it opens what it counts: the
/// counts share the row their own text ends on.
fn record_count_hits(area: Rect, props: FooterProps<'_>, lines: &[Line<'static>]) {
    if props.counts.is_empty() {
        return;
    }
    let Some(text) = props
        .counts
        .iter()
        .map(SideCount::text)
        .reduce(|all, text| format!("{all} · {text}"))
    else {
        return;
    };
    let Some(row) = lines
        .iter()
        .position(|line| line.width() >= text.width() && line.to_string().contains(&text))
    else {
        return;
    };
    let width = area.width.saturating_sub(FOOTER_INDENT_COLS as u16);
    let mut hits = props.hits.borrow_mut();
    hits.clear();
    let mut x = area.x + width.saturating_sub(text.width() as u16);
    for (index, count) in props.counts.iter().enumerate() {
        let piece = count.text();
        let piece_width = piece.chars().count() as u16;
        hits.push((
            Rect::new(x, area.y + row as u16, piece_width, 1),
            count.item,
        ));
        x = x.saturating_add(piece_width);
        if index + 1 < props.counts.len() {
            x = x.saturating_add(" · ".chars().count() as u16);
        }
    }
}

fn wrapped_footer(props: FooterProps<'_>, width: u16) -> Vec<Line<'static>> {
    let width = width.saturating_sub(FOOTER_INDENT_COLS as u16).max(1);
    let (lines, right) = footer_sides(props, width);
    let mut arranged = lines;
    place_right(&mut arranged, width, props.overflow, right);
    arranged
        .into_iter()
        .flat_map(|line| {
            if line.width() <= usize::from(width) {
                return vec![line];
            }
            crate::wrapping::word_wrap_lines([line], usize::from(width))
        })
        .collect()
}

/// The footer's left side, a row each, and its right side: the counts and
/// the goal, `None` when there is neither.
fn footer_sides(props: FooterProps<'_>, width: u16) -> (Vec<Line<'static>>, Option<Line<'static>>) {
    let mut lines = footer_lines(props);
    // The items placed on the second row's left are a row of their own,
    // under the hints.
    if let Some(second) = props.second_row.cloned() {
        lines.push(second);
    }
    let mut right = Line::default();
    for (index, count) in props.counts.iter().enumerate() {
        if index > 0 {
            right.spans.push(" · ".dim());
        }
        right.spans.push(count.text().dim());
    }
    if let Some(goal) = goal_line(props) {
        if !right.spans.is_empty() {
            right.spans.push(" · ".dim());
        }
        right.spans.extend(goal.spans);
    }
    let right = (!right.spans.is_empty() && right.width() <= usize::from(width)).then_some(right);
    (lines, right)
}

/// Puts `right` at the row's right edge, keeping the left row where the
/// overflow rule says it goes.
fn place_right(
    lines: &mut Vec<Line<'static>>,
    width: u16,
    overflow: Overflow,
    right: Option<Line<'static>>,
) {
    let Some(right) = right else {
        return;
    };
    let width = usize::from(width);
    let Some(left) = lines.last_mut() else {
        lines.push(right);
        return;
    };
    let used = left.width() + right.width();
    if used + 2 <= width {
        left.push_span(" ".repeat(width - used));
        left.spans.extend(right.spans);
        return;
    }
    match overflow {
        // The right side keeps the row, and the left side moves under it.
        Overflow::WrapLeft => {
            let left = std::mem::take(left);
            *lines.last_mut().expect("the row is there") = right;
            lines.push(left);
        }
        // Both sides move: the right side keeps its own row, the left side
        // follows it.
        Overflow::WrapAll => lines.push(right),
        // The left side is cut where the right side begins, and shares the
        // right side's row.
        Overflow::Truncate => {
            let room = width.saturating_sub(right.width() + 2).max(1);
            let mut row = Line::default();
            let mut used = 0;
            for span in std::mem::take(&mut left.spans) {
                if used >= room {
                    break;
                }
                let mut text = String::new();
                for ch in span.content.chars() {
                    let width_of = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    if used + width_of > room {
                        break;
                    }
                    text.push(ch);
                    used += width_of;
                }
                if text.is_empty() {
                    break;
                }
                row.spans
                    .push(ratatui::text::Span::styled(text, span.style));
            }
            let pad = width.saturating_sub(row.width() + right.width());
            row.push_span(" ".repeat(pad));
            row.spans.extend(right.spans);
            *lines.last_mut().expect("the row is there") = row;
        }
    }
}

fn goal_line(props: FooterProps<'_>) -> Option<Line<'static>> {
    if matches!(
        props.mode,
        FooterMode::QuitShortcutReminder | FooterMode::EscHint
    ) {
        return None;
    }
    props.goal?.line(std::time::Instant::now())
}

fn render_footer_row(area: Rect, buf: &mut Buffer, left: Line<'static>, right: Line<'static>) {
    let right_width = right.width().min(usize::from(area.width)) as u16;
    let left_width = area.width.saturating_sub(right_width).saturating_sub(2);
    buf.set_line(area.x, area.y, &left, left_width);
    buf.set_line(area.right() - right_width, area.y, &right, right_width);
}

impl super::BottomPane {
    /// The footer's sides beside the hints, and what it does when a row does
    /// not fit beside them.
    pub(crate) fn set_counts(&mut self, sides: FooterSides) {
        self.composer.set_counts(sides);
        self.request_redraw();
    }

    /// The footer count the pointer is on, when no view has the keys.
    pub(crate) fn footer_side_at(
        &self,
        mouse: ratatui::crossterm::event::MouseEvent,
    ) -> Option<SideItem> {
        if self.view_stack.last().is_some() {
            return None;
        }
        self.composer.footer_side_at(mouse)
    }

    #[cfg(test)]
    pub(crate) fn set_footer_hits_for_test(&mut self, hits: Vec<(Rect, SideItem)>) {
        self.composer.set_footer_hits_for_test(hits);
    }

    /// The composer's own pointer keys, taken before the footer's: a click
    /// in the text or a wheel turn there.
    pub(crate) fn handle_textarea_mouse(
        &mut self,
        mouse: ratatui::crossterm::event::MouseEvent,
    ) -> bool {
        if self.view_stack.last().is_some() || self.composer.popup_active() {
            return false;
        }
        self.composer.handle_textarea_mouse(mouse)
    }
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
    let mut lines: Vec<_> = props.status_line.cloned().into_iter().collect();
    if let Some(navigation) = props.navigation.filter(|_| {
        !matches!(
            props.mode,
            FooterMode::QuitShortcutReminder | FooterMode::EscHint
        )
    }) {
        lines.extend_from_slice(navigation);
        return lines;
    }
    lines.extend(match props.mode {
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
            _ => composer_empty_line(props),
        }],
        FooterMode::EscHint => vec![Line::from(vec![
            keymap::hint(Action::EditPrevious)
                .unwrap_or(key_hint::plain(KeyCode::Esc))
                .into(),
            " again to edit a previous message".into(),
        ])
        .dim()],
    });
    lines
}

/// The available shortcuts below the session information.
fn composer_empty_line(props: FooterProps<'_>) -> Line<'static> {
    let agents = [key_hint::plain(KeyCode::Left).into(), " for agents".dim()];
    let shortcuts = keymap::hint(Action::ShowShortcuts)
        .map(|key| vec![key.into(), " for shortcuts".dim()])
        .unwrap_or_default();
    let mut spans = if props.agents_hint {
        agents.to_vec()
    } else {
        Vec::new()
    };
    if !shortcuts.is_empty() {
        if !spans.is_empty() {
            spans.push(" · ".dim());
        }
        spans.extend(shortcuts);
    }
    Line::from(spans)
}

fn quit_shortcut_reminder_line(key: KeyBinding) -> Line<'static> {
    Line::from(vec![key.into(), " again to quit".into()]).dim()
}

#[cfg(test)]
mod tests;
