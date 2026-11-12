// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The command center (Codex's "Agent command center",
//! `app/agents_overview*.rs` and `app/agent_center/`), opened by `/agents`,
//! by Left on an empty composer and by `global.open_agents`: the sessions
//! without a parent, each with the status of the most urgent of itself and
//! its children (Needs you, Working, Ready, Inactive, and Error for a root
//! whose own turn failed), under headings by project, directory, status or
//! model (`g`, kept in `tui.agentsGrouping`), filtered by status (Tab) or
//! by the title (`/`), and the actions on the one selected: open it, start
//! a new session (in a new worktree with `w`), fork, rename, stop it,
//! archive or delete it (each after a confirmation).
//!
//! It is drawn over the whole screen, as the resume picker is, and keeps
//! the alternate screen where Codex's is a full-height bottom pane. It reads
//! pages of the session list when it needs them ([`AgentsCenter::page_request`]),
//! each listed session's children ([`AgentsCenter::children_wanted`]) and
//! whether a running one waits on an approval
//! ([`AgentsCenter::approvals_wanted`]); the app reads them and carries out
//! the actions ([`CenterAction`]). Codex's cloud tasks, usage, voice and
//! server notices are not ported.

mod grouping;
mod input;
mod render;
mod status;

use std::collections::HashMap;

use hanafubuki_agent::{
    AgentAgentsGrouping, AgentSessionId, AgentSessionListRequest, AgentSessionPage,
    AgentSessionParentFilter, AgentSessionSummary, AgentTurnPhase,
};

use grouping::{Heading, Place};
use status::{Filter, Status};

const PAGE_SIZE: usize = 25;
/// How many of a session's children are read for its status.
const CHILDREN_PAGE: usize = 50;
const LOAD_NEAR_THRESHOLD: usize = 5;

/// What the person asked of the center, for the app to carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CenterAction {
    Close,
    /// Shows the session in place of the current one.
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
    /// Interrupts the turn the session runs.
    Stop(AgentSessionId),
    /// The grouping changed: the app keeps it in `tui.agentsGrouping`.
    Grouping(AgentAgentsGrouping),
}

/// One listed session, with its status and its heading.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) summary: AgentSessionSummary,
    pub(crate) status: Status,
    pub(crate) heading: Heading,
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

pub(crate) struct AgentsCenter {
    filter: Filter,
    grouping: AgentAgentsGrouping,
    mode: Mode,
    roots: Vec<AgentSessionSummary>,
    children: HashMap<AgentSessionId, Vec<AgentSessionSummary>>,
    /// The running sessions read for an approval, and whether one waits.
    approvals: HashMap<AgentSessionId, bool>,
    /// Each directory's headings, read once.
    places: HashMap<String, Place>,
    loaded: bool,
    next_cursor: Option<String>,
    error: Option<String>,
    selected: Option<AgentSessionId>,
    scroll_top: usize,
    view_rows: Option<usize>,
    /// The session the interface shows.
    current: Option<AgentSessionId>,
    notice: Option<String>,
}

impl AgentsCenter {
    pub(crate) fn new(current: Option<AgentSessionId>, grouping: AgentAgentsGrouping) -> Self {
        Self {
            filter: Filter::All,
            grouping,
            mode: Mode::Browse,
            roots: Vec::new(),
            children: HashMap::new(),
            approvals: HashMap::new(),
            places: HashMap::new(),
            loaded: false,
            next_cursor: None,
            error: None,
            selected: current,
            scroll_top: 0,
            view_rows: None,
            current,
            notice: None,
        }
    }

    /// Says `notice` until the next key.
    pub(crate) fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    /// The list read again from its first page (after an action changed
    /// it), the selection kept on its session.
    pub(crate) fn reload(&mut self, current: Option<AgentSessionId>) {
        self.current = current;
        self.roots.clear();
        self.children.clear();
        self.approvals.clear();
        self.loaded = false;
        self.next_cursor = None;
        self.error = None;
    }

    /// Whether a key types text here (a search or a rename), so that no
    /// chord waits on it.
    pub(crate) fn takes_text(&self) -> bool {
        matches!(self.mode, Mode::Search(_) | Mode::Rename { .. })
    }

    /// The rows shown, in order: the sessions the filter admits, under
    /// their headings (Codex's order: by heading, the latest first within
    /// one; by status, the most urgent first, then the latest).
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

    /// Every loaded session as a row, whatever the filter.
    fn all_rows(&self) -> Vec<Row> {
        self.roots
            .iter()
            .map(|root| {
                let children = self.children.get(&root.session_id).into_iter().flatten();
                let status = Status::of(root, children, |session| {
                    self.approvals
                        .get(&session.session_id)
                        .copied()
                        .unwrap_or(false)
                });
                let place = root.cwd.as_ref().and_then(|cwd| self.places.get(cwd));
                Row {
                    heading: Heading::of(self.grouping, root, status, place),
                    summary: root.clone(),
                    status,
                }
            })
            .collect()
    }

    /// The page of sessions the center needs next: the first one, or the
    /// next while the selection nears the end of the rows or they do not
    /// fill the list. `None` when nothing more is needed.
    pub(crate) fn page_request(&self) -> Option<AgentSessionListRequest> {
        if self.error.is_some() {
            return None;
        }
        let cursor = if self.loaded {
            let cursor = self.next_cursor.clone()?;
            let rows = self.rows();
            let remaining = rows.len().saturating_sub(self.selected_index(&rows) + 1);
            let unfilled = rows.len() < self.view_rows.unwrap_or(PAGE_SIZE);
            if remaining > LOAD_NEAR_THRESHOLD && !unfilled {
                return None;
            }
            Some(cursor)
        } else {
            None
        };
        Some(AgentSessionListRequest {
            page_size: PAGE_SIZE,
            cursor,
            lifecycle: self.filter.lifecycle(),
            search: self.query().map(str::to_string),
            ..AgentSessionListRequest::default()
        })
    }

    pub(crate) fn ingest_page(&mut self, page: Result<AgentSessionPage, String>) {
        let page = match page {
            Ok(page) => page,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.loaded = true;
        self.next_cursor = page.next_cursor;
        // A session no turn ran in has nothing to show, as in the picker.
        for session in page.sessions {
            if session.latest_turn_phase.is_none() {
                continue;
            }
            if let Some(cwd) = &session.cwd {
                if !self.places.contains_key(cwd) {
                    self.places.insert(cwd.clone(), Place::of(cwd));
                }
            }
            self.roots.push(session);
        }
    }

    /// The listed sessions whose children are not read yet, with the
    /// request that reads them (never searched: the search is the roots').
    pub(crate) fn children_wanted(&self) -> Vec<(AgentSessionId, AgentSessionListRequest)> {
        self.roots
            .iter()
            .filter(|root| !self.children.contains_key(&root.session_id))
            .map(|root| {
                let request = AgentSessionListRequest {
                    page_size: CHILDREN_PAGE,
                    lifecycle: self.filter.lifecycle(),
                    parent: AgentSessionParentFilter::ChildrenOf(root.session_id),
                    ..AgentSessionListRequest::default()
                };
                (root.session_id, request)
            })
            .collect()
    }

    /// `parent`'s children; a failure reads as none.
    pub(crate) fn ingest_children(
        &mut self,
        parent: AgentSessionId,
        page: Result<AgentSessionPage, String>,
    ) {
        let children = page.map(|page| page.sessions).unwrap_or_default();
        self.children.insert(parent, children);
    }

    /// The listed sessions, roots and children, whose turn runs and which
    /// were not read yet for an approval waiting.
    pub(crate) fn approvals_wanted(&self) -> Vec<AgentSessionId> {
        self.roots
            .iter()
            .chain(self.children.values().flatten())
            .filter(|session| runs(session) && !self.approvals.contains_key(&session.session_id))
            .map(|session| session.session_id)
            .collect()
    }

    pub(crate) fn ingest_approval(&mut self, session_id: AgentSessionId, waiting: bool) {
        self.approvals.insert(session_id, waiting);
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
