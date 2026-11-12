// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The command center (Codex's "Agent command center",
//! `app/agents_overview*.rs` and `app/agent_center/`), opened by `/agents`,
//! by Left on an empty composer and by `global.open_agents`: the sessions
//! without a parent, each with the status of the most urgent of itself and
//! its children (Needs you, Working, Ready, Inactive, and Error for a root
//! whose own turn failed), under headings by project, directory, status or
//! model (`g`, kept in `tui.agentsGrouping`), the pinned ones first,
//! filtered by status (Tab) or by the title (`/`), and the actions on the
//! one selected: open it, start a new session (in a new worktree with `w`),
//! fork, rename, stop it, pin or hide it, archive or delete it (each after a
//! confirmation). A details pane beside the list (from 90 columns) says
//! more of the one selected and lists its children, which open and stop
//! from there.
//!
//! It is drawn over the whole screen, as the resume picker is, and keeps
//! the alternate screen where Codex's is a full-height bottom pane. What it
//! shows is read off the interface's loop ([`AgentsCenter::jobs`], answered
//! through [`AgentsCenter::ingest`]); the sessions the Agent holds are
//! watched while it is open, and those that changed are read again. The app
//! carries out the actions ([`CenterAction`]) and keeps the pins, the hidden
//! sessions and when each was last shown ([`AgentsMarks`]). Codex's cloud
//! tasks, usage in credits, voice and server notices are not ported.

mod details;
mod grouping;
mod input;
mod loading;
mod mouse;
mod render;
mod status;

use std::collections::HashMap;

use hanafubuki_agent::{
    AgentAgentsGrouping, AgentRevision, AgentSessionId, AgentSessionSummary, AgentTurnPhase,
};
use ratatui::layout::Rect;

use crate::agent_bridge::{Exchange, LiveSession, Waiting};
use crate::tui_state::AgentsMarks;
use grouping::{Heading, Place};
use loading::Reads;
use status::{Filter, Group, Status};

/// What the person asked of the center, for the app to carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CenterAction {
    Close,
    /// Shows the session in place of the current one (a child beside its
    /// parent).
    Open(Box<AgentSessionSummary>),
    New,
    /// A new session in a new worktree, as `/worktree` starts one.
    NewWorktree,
    Fork(AgentSessionId),
    Rename {
        session_id: AgentSessionId,
        title: String,
    },
    SetArchived {
        session_id: AgentSessionId,
        archive: bool,
    },
    Delete(AgentSessionId),
    /// Interrupts the turn the session (a root or a child) runs.
    Stop(Box<AgentSessionSummary>),
    /// The grouping changed: the app keeps it in `tui.agentsGrouping`.
    Grouping(AgentAgentsGrouping),
    /// The pins or the hidden sessions changed: the app keeps them.
    Marks(AgentsMarks),
}

/// One listed session, with its status and its heading.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) summary: AgentSessionSummary,
    pub(crate) status: Status,
    pub(crate) heading: Heading,
    /// Listed only while a search runs.
    pub(crate) hidden: bool,
    /// It or a child changed since it was last shown.
    pub(crate) unread: bool,
}

/// What the center is doing besides listing.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    Browse,
    /// The title searched for, as typed.
    Search(String),
    Rename {
        session_id: AgentSessionId,
        title: String,
    },
    Confirm(Confirm),
    Help,
}

/// An archive or a delete waiting for its confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Confirm {
    session_id: AgentSessionId,
    title: String,
    delete: bool,
    /// The action is chosen; else Cancel, the default.
    accept: bool,
}

/// Where the last frame drew what the mouse reaches.
#[derive(Clone, Debug, Default)]
struct ScreenLayout {
    tabs: Vec<(Rect, Filter)>,
    /// The session rows.
    rows: Vec<(Rect, AgentSessionId)>,
    /// The details pane's children.
    children: Vec<(Rect, AgentSessionId)>,
}

pub(crate) struct AgentsCenter {
    filter: Filter,
    grouping: AgentAgentsGrouping,
    mode: Mode,
    roots: Vec<AgentSessionSummary>,
    children: HashMap<AgentSessionId, Vec<AgentSessionSummary>>,
    /// The sessions the Agent holds, as last watched.
    live: HashMap<AgentSessionId, LiveSession>,
    /// Each session's latest exchange, and its revision when read.
    exchanges: HashMap<AgentSessionId, (Option<AgentRevision>, Exchange)>,
    reads: Reads,
    marks: AgentsMarks,
    /// Each directory's headings, read once.
    places: HashMap<String, Place>,
    error: Option<String>,
    selected: Option<AgentSessionId>,
    /// The child selected in the details pane, which then has the keys.
    child_selected: Option<AgentSessionId>,
    scroll_top: usize,
    view_rows: Option<usize>,
    layout: ScreenLayout,
    /// The session the interface shows.
    current: Option<AgentSessionId>,
    notice: Option<String>,
}

impl AgentsCenter {
    pub(crate) fn new(
        current: Option<AgentSessionId>,
        grouping: AgentAgentsGrouping,
        marks: AgentsMarks,
    ) -> Self {
        Self {
            filter: Filter::All,
            grouping,
            mode: Mode::Browse,
            roots: Vec::new(),
            children: HashMap::new(),
            live: HashMap::new(),
            exchanges: HashMap::new(),
            reads: Reads::default(),
            marks,
            places: HashMap::new(),
            error: None,
            selected: current,
            child_selected: None,
            scroll_top: 0,
            view_rows: None,
            layout: ScreenLayout::default(),
            current,
            notice: None,
        }
    }

    /// Says `notice` until the next key.
    pub(crate) fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    /// The marks as the app keeps them now.
    pub(crate) fn set_marks(&mut self, marks: AgentsMarks) {
        self.marks = marks;
    }

    /// Whether a key types text here (a search or a rename), so that no
    /// chord waits on it.
    pub(crate) fn takes_text(&self) -> bool {
        matches!(self.mode, Mode::Search(_) | Mode::Rename { .. })
    }

    /// The rows shown, in order: the sessions the filter admits, under
    /// their headings (Codex's order: the pinned first, then by heading,
    /// the latest first within one; by status, the most urgent first).
    pub(crate) fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .all_rows()
            .into_iter()
            .filter(|row| self.filter.admits(row.status))
            .collect();
        rows.sort_by(|left, right| {
            left.heading
                .key
                .cmp(&right.heading.key)
                .then_with(|| right.summary.updated_at.cmp(&left.summary.updated_at))
        });
        rows
    }

    /// Every loaded session as a row, whatever the filter; a hidden one
    /// only while a search runs.
    fn all_rows(&self) -> Vec<Row> {
        let searching = self.query().is_some();
        self.roots
            .iter()
            .filter(|root| searching || !self.marks.hidden.contains(&root.session_id))
            .map(|root| {
                let id = root.session_id;
                let children = self.children.get(&id).into_iter().flatten();
                let updated_at = children
                    .clone()
                    .map(|child| child.updated_at)
                    .fold(root.updated_at, u128::max);
                let status = Status::of(root, children, |session| self.waits_approval(session));
                let place = root.cwd.as_ref().and_then(|cwd| self.places.get(cwd));
                let pinned = self.marks.pinned.contains(&id);
                Row {
                    heading: if pinned {
                        Heading::pinned()
                    } else {
                        Heading::of(self.grouping, root, status, place)
                    },
                    summary: root.clone(),
                    status,
                    hidden: self.marks.hidden.contains(&id),
                    unread: Some(id) != self.current && self.marks.unread(id, updated_at),
                }
            })
            .collect()
    }

    /// Whether the session's running turn waits on an approval, as the
    /// watch last saw it.
    fn waits_approval(&self, session: &AgentSessionSummary) -> bool {
        self.live
            .get(&session.session_id)
            .is_some_and(|live| matches!(live.waiting, Some(Waiting::Approval(_))))
    }

    /// The search typed, while there is one.
    fn query(&self) -> Option<&str> {
        match &self.mode {
            Mode::Search(query) if !query.is_empty() => Some(query),
            _ => None,
        }
    }

    /// Where the selection is among `rows`: on its session, else the first.
    fn selected_index(&self, rows: &[Row]) -> usize {
        self.selected
            .and_then(|selected| {
                rows.iter()
                    .position(|row| row.summary.session_id == selected)
            })
            .unwrap_or(0)
    }

    fn selected_row(&self) -> Option<Row> {
        let rows = self.rows();
        let index = self.selected_index(&rows);
        rows.into_iter().nth(index)
    }

    /// The children of the session `root`, as listed.
    fn children_of(&self, root: AgentSessionId) -> &[AgentSessionSummary] {
        self.children.get(&root).map_or(&[], Vec::as_slice)
    }

    /// The child selected in the details pane, while it has the keys.
    fn selected_child(&self) -> Option<&AgentSessionSummary> {
        let child = self.child_selected?;
        let root = self.selected_row()?.summary.session_id;
        self.children_of(root)
            .iter()
            .find(|summary| summary.session_id == child)
    }

    /// The session the details pane says the most of: the most urgent of
    /// the root and its children, the root on a tie.
    fn most_urgent<'a>(&'a self, row: &'a Row) -> &'a AgentSessionSummary {
        let group =
            |session: &AgentSessionSummary| Group::of(session, self.waits_approval(session));
        self.children_of(row.summary.session_id)
            .iter()
            .fold(&row.summary, |most, child| {
                if group(child) < group(most) {
                    child
                } else {
                    most
                }
            })
    }
}

/// Whether `session`'s turn runs.
fn runs(session: &AgentSessionSummary) -> bool {
    matches!(
        session.latest_turn_phase,
        Some(AgentTurnPhase::Sampling | AgentTurnPhase::RunningTools)
    )
}

#[cfg(test)]
mod tests;
