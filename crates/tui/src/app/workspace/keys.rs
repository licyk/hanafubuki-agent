// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The panes' keys (Codex's pane actions under its workspace leader): each
//! is a keymap action (`global.focus_pane_left` and the rest), bound by
//! default to the leader, Ctrl+X, then a key; the keymap's chords read
//! them, so `tui.keymap` and `/keymap` rebind them.
//!
//! And the step through the agents (Codex's Alt+Left and Alt+Right,
//! `previous_agent` and `next_agent` on an empty composer): the main
//! session, then its subagents in `/subagents`' order, round. A subagent is
//! shown as `/subagents` shows it, the main session as `focus_pane_left`
//! returns to it.

use std::io::{self, Write};

use hanafubuki_agent::AgentSessionId;
use ratatui::backend::Backend;
use ratatui::crossterm::event::KeyEvent;

use super::{Focus, SecondKind};
use crate::app::App;
use crate::keymap::{self, Action};
use crate::tui::Tui;

const NO_SUBAGENTS: &str = "This session has no subagents.";

impl App {
    /// Lists the main session's subagents for a step; what is shown moves
    /// once they are listed.
    pub(super) fn list_agents_to_step(&mut self, forward: bool) {
        let Some(parent) = self.main_session_id() else {
            return self.chat_widget.add_info_message(NO_SUBAGENTS.to_string());
        };
        // The main conversation's bridge, parked while a second one is shown.
        let bridge = match self.workspace.second.as_ref() {
            Some(second) if second.focus == Focus::Second => &second.parked.bridge,
            _ => &self.bridge,
        };
        bridge.list_subagents_to_step(parent, forward);
    }

    /// Shows the agent after (or before) the one shown, among `parent` and
    /// its subagents `listed`, unless the main conversation moved meanwhile.
    pub(super) fn step_agent<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        parent: AgentSessionId,
        forward: bool,
        listed: Result<Vec<AgentSessionId>, String>,
    ) {
        if self.main_session_id() != Some(parent) {
            return;
        }
        let children = match listed {
            Ok(children) => children,
            Err(error) => {
                return self
                    .chat_widget
                    .add_error_message(format!("The subagents were not listed: {error}"))
            }
        };
        let shown = match self.workspace.second.as_ref() {
            Some(second) if second.focus == Focus::Second => match second.kind {
                SecondKind::Child { session, .. } => session,
                SecondKind::Side { .. } => parent,
            },
            _ => parent,
        };
        match step_target(parent, &children, shown, forward) {
            None => self.chat_widget.add_info_message(NO_SUBAGENTS.to_string()),
            Some(target) if target == parent => self.return_to_main(tui),
            Some(target) => self.open_child(tui, target),
        }
    }
}

/// Among `main` then its `children`, the one after (`forward`) or before
/// `shown`, round; `None` with no child.
pub(super) fn step_target(
    main: AgentSessionId,
    children: &[AgentSessionId],
    shown: AgentSessionId,
    forward: bool,
) -> Option<AgentSessionId> {
    if children.is_empty() {
        return None;
    }
    let order: Vec<AgentSessionId> = std::iter::once(main)
        .chain(children.iter().copied())
        .collect();
    let at = order
        .iter()
        .position(|session| *session == shown)
        .unwrap_or(0);
    let step = if forward { 1 } else { order.len() - 1 };
    Some(order[(at + step) % order.len()])
}

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
