// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The panes' keys (Codex's pane actions under its workspace leader): each
//! is a keymap action (`global.focus_pane_left` and the rest), bound by
//! default to the leader, Ctrl+X, then a key; the keymap's chords read
//! them, so `tui.keymap` and `/keymap` rebind them.

use ratatui::crossterm::event::KeyEvent;

use crate::keymap::{self, Action};

/// What the panes' keys do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneAction {
    /// The main conversation.
    FocusMain,
    /// The second conversation.
    FocusSecond,
    /// The other conversation.
    ToggleFocus,
    /// The side pane grows.
    DividerLeft,
    /// The side pane shrinks.
    DividerRight,
    EqualWidths,
    /// The focused pane alone, or the split again.
    ToggleZoom,
    /// Closes the side conversation, or leaves the subagent.
    Close,
}

impl PaneAction {
    /// Each action, its keymap action, and what the HUD calls it.
    pub(super) const ALL: [(Self, Action, &'static str); 8] = [
        (Self::FocusMain, Action::FocusPaneLeft, "main conversation"),
        (Self::FocusSecond, Action::FocusPaneRight, "side pane"),
        (
            Self::ToggleFocus,
            Action::ToggleSideConversation,
            "other pane",
        ),
        (Self::DividerLeft, Action::MoveDividerLeft, "divider left"),
        (
            Self::DividerRight,
            Action::MoveDividerRight,
            "divider right",
        ),
        (
            Self::EqualWidths,
            Action::EqualizePaneWidths,
            "equal widths",
        ),
        (Self::ToggleZoom, Action::TogglePaneZoom, "zoom pane"),
        (Self::Close, Action::HidePane, "close the side pane"),
    ];

    /// The action `key` (a key, or a completed chord's token) triggers.
    pub(super) fn of(key: &KeyEvent) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|(_, action, _)| keymap::matches(*action, key))
            .map(|(pane, _, _)| pane)
    }
}

/// `action`'s first keys as a hint says them: `ctrl-x right` for a leader
/// chord.
pub(super) fn hint(action: Action) -> String {
    let keymap = keymap::current();
    let leader = keymap.label(Action::Leader);
    let keys = keymap.label(action);
    let first = keys.split(", ").next().unwrap_or_default();
    match first.strip_prefix("leader ") {
        Some(second) => format!("{leader} {second}"),
        None => first.to_string(),
    }
}

/// The HUD's lines: each action's keys as the keymap has them, the leader
/// left out (it was just pressed), and what it does.
pub(super) fn hud_entries() -> Vec<(String, &'static str)> {
    let keymap = keymap::current();
    PaneAction::ALL
        .into_iter()
        .map(|(_, action, label)| {
            let keys = keymap.label(action).replace("leader ", "");
            (keys, label)
        })
        .collect()
}
