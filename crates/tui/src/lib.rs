//! The terminal interface of the standalone Agent (`hanafubuki-agent`
//! with no command), ported from the Codex TUI at `rust-v0.93.0`: an inline
//! viewport at the bottom of the terminal with the composer, the status
//! line and the approval and question overlays, and the transcript written
//! into the terminal's own scrollback above it. It drives the Agent through
//! the facade alone (`agent_bridge`).

mod agent_bridge;
mod app;
mod app_event;
mod bottom_pane;
mod chatwidget;
mod commands;
mod custom_terminal;
mod diff_render;
mod file_search;
mod get_git_diff;
mod history_cell;
mod insert_history;
mod interface;
mod key_hint;
mod markdown;
mod markdown_render;
mod markdown_stream;
mod pager_overlay;
mod render;
mod resume_picker;
mod shimmer;
mod status_indicator_widget;
mod streaming;
mod style;
#[cfg(test)]
mod test_support;
mod text_formatting;
#[allow(dead_code, reason = "P2.2 wires the transcript view into the app")]
mod text_selection;
#[allow(dead_code, reason = "P2.2 wires the transcript view into the app")]
mod transcript_view;
mod tui;
mod ui_consts;
mod wrapping;

pub use interface::{run, TuiEvents, TuiOptions, TuiResume};
