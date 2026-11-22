// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Shared UI constants for layout and alignment within the TUI.

/// Width (in terminal columns) reserved for the left gutter/prefix used by
/// live cells and aligned widgets.
///
/// Semantics:
/// - Chat composer reserves this many columns for the left border + padding.
/// - Status indicator lines begin with this many spaces for alignment.
/// - User history lines account for this many columns (e.g., "▌ ") when wrapping.
pub(crate) const LIVE_PREFIX_COLS: u16 = 2;
pub(crate) const FOOTER_INDENT_COLS: usize = LIVE_PREFIX_COLS as usize;

/// Leave a third of the pane for reading, or allow eight composer rows on small screens.
pub(crate) fn pane_composer_height_limit(height: u16) -> u16 {
    ((u32::from(height) * 2 / 3) as u16).max(8).min(height)
}
