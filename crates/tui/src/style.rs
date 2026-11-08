// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The fixed theme's block styles. Codex tints the user's messages and the
//! proposed plan from the terminal's queried background; the terminal is not
//! queried here, so they take the terminal's own colours, as Codex's do when
//! its query gets no answer.

use ratatui::style::Style;

pub fn user_message_style() -> Style {
    Style::default()
}

pub fn proposed_plan_style() -> Style {
    Style::default()
}
