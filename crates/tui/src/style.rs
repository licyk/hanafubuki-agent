// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The fixed theme's block styles. Codex tints the user's messages and the
//! proposed plan from the terminal's queried background; the terminal is not
//! queried here, so they take the terminal's own colours, as Codex's do when
//! its query gets no answer.

use ratatui::style::{Color, Modifier, Style};

pub fn user_message_style() -> Style {
    Style::default()
}

pub fn proposed_plan_style() -> Style {
    Style::default()
}

/// Selected transcript text. Codex derives it from the queried background;
/// without the query it reverses the cell.
pub(crate) fn selection_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Emphasis for the transcript's controls.
pub(crate) fn accent_color() -> Color {
    Color::Cyan
}

/// A table's rules. Codex blends them from the queried foreground and
/// background; without the query it dims them.
pub(crate) fn table_separator_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}
