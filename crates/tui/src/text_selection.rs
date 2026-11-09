// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The selection gestures views share and the text units they select;
//! each view does its own hit testing and keeps its own state.

use std::ops::Range;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

/// How long two clicks at one cell may be apart to count as one gesture.
const MULTI_CLICK: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionUnit {
    Character,
    Word,
    Line,
}

impl SelectionUnit {
    pub(crate) fn from_clicks(clicks: u8) -> Self {
        match clicks {
            2 => Self::Word,
            3 => Self::Line,
            _ => Self::Character,
        }
    }

    /// The unit of `text` around `offset`.
    pub(crate) fn range(self, text: &str, offset: usize) -> Range<usize> {
        let offset = text.floor_char_boundary(offset.min(text.len()));
        match self {
            Self::Character => offset..offset,
            Self::Word => text
                .split_word_bound_indices()
                .map(|(start, word)| start..start + word.len())
                .find(|range| range.contains(&offset))
                .unwrap_or(text.len()..text.len()),
            // A line is a logical one, with its line break, however it wraps.
            Self::Line => {
                let start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
                let end = text[offset..]
                    .find('\n')
                    .map_or(text.len(), |newline| offset + newline + 1);
                start..end
            }
        }
    }
}

/// Counts this click with the ones just before it at the same cell: one,
/// two or three, then one again.
pub(crate) fn click_count(
    last_click: &mut Option<(Instant, u16, u16, u8)>,
    column: u16,
    row: u16,
) -> u8 {
    let now = Instant::now();
    let clicks = last_click
        .filter(|(at, x, y, _)| now.duration_since(*at) < MULTI_CLICK && *x == column && *y == row)
        .map_or(1, |(_, _, _, clicks)| clicks % 3 + 1);
    *last_click = Some((now, column, row, clicks));
    clicks
}

/// Ctrl+C, Cmd+C (which Kitty reports as Super+C), Ctrl+Shift+C, or
/// Ctrl+Insert (which Windows Terminal forwards when it has no selection of
/// its own).
pub(crate) fn is_copy_key(key: KeyEvent) -> bool {
    let (code, modifiers) = crate::key_hint::normalize_key_parts(key.code, key.modifiers);
    key.kind != KeyEventKind::Release
        && ((code == KeyCode::Char('c')
            && (matches!(modifiers, KeyModifiers::CONTROL | KeyModifiers::SUPER)
                || modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT)))
            || (code == KeyCode::Insert && modifiers == KeyModifiers::CONTROL))
}
