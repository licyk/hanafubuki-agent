//! The terminal interface of the standalone Agent (`hanafubuki-agent`
//! with no command), ported from the Codex TUI: the inline chat, its
//! composer, the status line and the approval and question overlays at
//! `rust-v0.93.0`, and the owned transcript at `57a808a392`, which draws
//! the conversation on the alternate screen in a transcript view with
//! scrolling, selection, search and copying. The transcript's mode is
//! chosen at launch: owned (Fullscreen) by default, or written into the
//! terminal's own scrollback (Scrollback). It drives the Agent through the
//! facade alone (`agent_bridge`).

mod agent_bridge;
mod app;
mod app_event;
mod appearance;
mod bottom_pane;
mod chatwidget;
mod clipboard;
mod commands;
mod custom_terminal;
mod diff_render;
mod file_search;
mod get_git_diff;
mod history_cell;
mod insert_history;
mod interface;
mod key_hint;
mod keymap;
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
mod text_selection;
mod transcript_mode;
mod transcript_view;
mod tui;
mod ui_consts;
mod wrapping;

pub use interface::{run, TuiEvents, TuiOptions, TuiResume};
