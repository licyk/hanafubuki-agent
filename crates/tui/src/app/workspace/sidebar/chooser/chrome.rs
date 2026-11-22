// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).
//! Chooser framing and its painted shortcut targets.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::FooterAction;
use crate::footer_hint::{shortcut, HintLine};
use crate::keymap::{self, Action};
use crate::text_formatting::strip_terminal_controls;

/// Leaves the tab strip's left margin and the scroll arrows' right margin.
pub(super) fn body_row(area: Rect, y: u16) -> Rect {
    Rect::new(area.x.saturating_add(1), y, area.width.saturating_sub(3), 1)
}

fn paint_offset(area: Rect, buf: &mut Buffer, top: usize, offset: usize, line: Line<'_>) {
    let Some(row) = offset.checked_sub(top) else {
        return;
    };
    if (1..1 + usize::from(area.height.saturating_sub(3))).contains(&row) {
        let mut rect = body_row(area, area.y + row as u16);
        rect.width = rect.width.saturating_sub(1);
        line.render(rect, buf);
    }
}

/// A fetching or error notice shares the row between the title and the list.
pub(super) fn header(area: Rect, buf: &mut Buffer, top: usize, status: Option<(&str, bool)>) {
    paint_offset(area, buf, top, 2, Line::from("Open in sidebar").bold());
    if let Some((message, failed)) = status {
        let line = Line::from(strip_terminal_controls(message));
        paint_offset(
            area,
            buf,
            top,
            3,
            if failed { line.red() } else { line.dim() },
        );
    }
}

pub(super) fn waiting(area: Rect, buf: &mut Buffer) {
    let width = usize::from(body_row(area, area.y).width.saturating_sub(1).max(1));
    for (index, text) in textwrap::wrap(
        "You can start a side chat after your first message in the main chat.",
        width,
    )
    .into_iter()
    .enumerate()
    {
        paint_offset(area, buf, 0, 4 + index, Line::from(text).dim());
    }
}

pub(super) fn arrows(area: Rect, buf: &mut Buffer, has_above: bool, has_below: bool) {
    if area.width < 3 || area.height <= 3 {
        return;
    }
    for (visible, y, arrow) in [
        (has_above, area.y + 1, "↑"),
        (has_below, area.bottom() - 2, "↓"),
    ] {
        if visible {
            Line::from(arrow)
                .dim()
                .render(Rect::new(area.right() - 2, y, 1, 1), buf);
        }
    }
}

pub(super) enum Footer<'a> {
    Browse {
        open: Option<&'a str>,
        file_path: bool,
        archive: bool,
        movement: bool,
    },
    Path,
}

pub(super) fn footer(area: Rect, buf: &mut Buffer, mode: Footer<'_>) -> Vec<(FooterAction, Rect)> {
    if area.height < 2 {
        return Vec::new();
    }
    let row = body_row(area, area.bottom() - 1);
    let mut hints = Vec::new();
    let mut push = |keys: &str, text: &str, action| hints.push((shortcut(keys, text), action));
    match mode {
        Footer::Path => {
            push("Esc", "back", Some(FooterAction::Back));
            push("↓", "scroll", None);
        }
        Footer::Browse {
            open,
            file_path,
            archive,
            movement,
        } => {
            if let Some(label) = open {
                push("Enter", label, Some(FooterAction::Open));
            }
            push("Esc", "back", Some(FooterAction::Back));
            if file_path {
                push("p", "path", Some(FooterAction::Path));
            }
            if archive {
                push("Del", "archive", Some(FooterAction::Archive));
            }
            if movement {
                push("↑/↓", "move", None);
            }
            if let Some(label) = keymap::current().primary_label(Action::HidePane) {
                push(&label, "hide pane", Some(FooterAction::Hide));
            }
        }
    }
    let (lines, actions): (Vec<_>, Vec<_>) = hints.into_iter().unzip();
    let hints = HintLine::new(lines, row.width);
    hints.line.render(row, buf);
    hints
        .items
        .into_iter()
        .zip(actions)
        .filter_map(|(range, action)| {
            Some((
                action?,
                Rect::new(row.x + range.start, row.y, range.end - range.start, 1),
            ))
        })
        .collect()
}

/// Returns the page height after wrapping at the actual panel width. Accepts
/// the existing scroll offset so shrinking or enlarging the pane clamps it.
pub(super) fn file_path(area: Rect, buf: &mut Buffer, path: &str, top: &mut usize) -> usize {
    let mut body = body_row(area, area.y.saturating_add(u16::from(area.height > 2) * 2));
    body.height = area
        .height
        .saturating_sub(u16::from(area.height > 2) * 2 + 1);
    let header_rows = u16::from(body.height > 2) * 2;
    let height = usize::from(body.height - header_rows);
    let path = strip_terminal_controls(path);
    let rows = textwrap::wrap(&path, usize::from(body.width.max(1)));
    *top = (*top).min(rows.len().saturating_sub(height));
    let mut lines = Vec::new();
    if header_rows > 0 {
        lines.extend([Line::from("File path").bold(), Line::default()]);
    }
    lines.extend(rows.into_iter().skip(*top).take(height).map(Line::from));
    Paragraph::new(lines).render(body, buf);
    height
}
