//! The terminal interface.

mod agent_bridge;
mod app_event;
// Wired by the chat widget.
#[allow(dead_code)]
mod bottom_pane;
mod custom_terminal;
mod diff_render;
mod history_cell;
mod insert_history;
mod key_hint;
mod markdown;
mod markdown_render;
mod markdown_stream;
mod render;
mod shimmer;
mod status_indicator_widget;
mod streaming;
mod style;
#[cfg(test)]
mod test_support;
mod text_formatting;
mod tui;
mod ui_consts;
mod wrapping;
