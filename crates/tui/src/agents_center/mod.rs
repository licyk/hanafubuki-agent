// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The command center `/agents` opens (Codex's agents overview,
//! `app/agents_overview*.rs` and `app/agent_center/`): the sessions, the
//! latest first, each followed by its children (subagents, reviews), with
//! a status filter (all, running, idle, failed, archived) and the actions
//! on the one selected: open it, start a new session, fork it, archive or
//! restore it, delete it (after a confirmation), interrupt what it runs.
//!
//! It is drawn over the whole screen, as the resume picker is. Like the
//! picker, it reads pages of the session list when it needs them
//! ([`AgentsCenter::page_request`]) and each listed session's children
//! ([`AgentsCenter::children_wanted`]); the app reads them and carries out
//! the actions ([`CenterAction`]). Codex's cloud tasks, daemon hosts,
//! usage, and its own threads' grouping by project are not ported.

mod render;

use std::collections::HashMap;

use hanafubuki_agent::{
    AgentSessionId, AgentSessionLifecycle, AgentSessionListRequest, AgentSessionPage,
    AgentSessionParentFilter, AgentSessionSummary, AgentTurnPhase,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const PAGE_SIZE: usize = 25;
/// How many of a session's children are listed under it.
const CHILDREN_PAGE: usize = 50;
const LOAD_NEAR_THRESHOLD: usize = 5;

/// Which sessions the center lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CenterFilter {
    All,
    Running,
    Idle,
    Failed,
    Archived,
}

impl CenterFilter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Running,
        Self::Idle,
        Self::Failed,
        Self::Archived,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Running => "Running",
            Self::Idle => "Idle",
            Self::Failed => "Failed",
            Self::Archived => "Archived",
        }
    }

    fn lifecycle(self) -> AgentSessionLifecycle {
        match self {
            Self::Archived => AgentSessionLifecycle::Archived,
            _ => AgentSessionLifecycle::Available,
        }
    }

    fn admits(self, status: SessionStatus) -> bool {
        match self {
            Self::All | Self::Archived => true,
            Self::Running => status == SessionStatus::Running,
            Self::Idle => status == SessionStatus::Idle,
            Self::Failed => status == SessionStatus::Failed,
        }
    }

    fn step(self, forward: bool) -> Self {
        let index = Self::ALL
            .iter()
            .position(|filter| *filter == self)
            .unwrap_or(0);
        let count = Self::ALL.len();
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        Self::ALL[next]
    }
}

/// A session's state as the list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionStatus {
    Running,
    Idle,
    Failed,
}

impl SessionStatus {
    pub(crate) fn of(summary: &AgentSessionSummary) -> Self {
        match summary.latest_turn_phase {
            Some(
                AgentTurnPhase::Sampling
                | AgentTurnPhase::RunningTools
                | AgentTurnPhase::WaitingForUserInput,
            ) => Self::Running,
            Some(AgentTurnPhase::Failed) => Self::Failed,
            _ => Self::Idle,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Failed => "failed",
        }
    }
}

/// What the person asked of the center, for the app to carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CenterAction {
    Close,
    /// Shows the session: a top-level one in place of the current, a child
    /// beside its parent.
    Open(AgentSessionSummary),
    New,
    Fork(AgentSessionId),
    SetArchived {
        session_id: AgentSessionId,
        archive: bool,
    },
    Delete(AgentSessionId),
    /// Interrupts the turn the session runs (a subagent's through the
    /// subagents' own call).
    Interrupt(AgentSessionSummary),
}

/// One listed session: a top-level one, or (`child`) one of its children.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) summary: AgentSessionSummary,
    pub(crate) child: bool,
}

pub(crate) struct AgentsCenter {
    filter: CenterFilter,
    parents: Vec<AgentSessionSummary>,
    children: HashMap<AgentSessionId, Vec<AgentSessionSummary>>,
    loaded: bool,
    next_cursor: Option<String>,
    error: Option<String>,
    selected: usize,
    scroll_top: usize,
    view_rows: Option<usize>,
    /// The session the interface shows.
    current: Option<AgentSessionId>,
    /// The session Delete asks about, until the next key.
    confirm_delete: Option<AgentSessionId>,
    notice: Option<String>,
}

impl AgentsCenter {
    pub(crate) fn new(current: Option<AgentSessionId>) -> Self {
        Self {
            filter: CenterFilter::All,
            parents: Vec::new(),
            children: HashMap::new(),
            loaded: false,
            next_cursor: None,
            error: None,
            selected: 0,
            scroll_top: 0,
            view_rows: None,
            current,
            confirm_delete: None,
            notice: None,
        }
    }

    /// Says `notice` until the next key.
    pub(crate) fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    /// The list read again from its first page (after an action changed
    /// it), the selection kept where it was.
    pub(crate) fn reload(&mut self, current: Option<AgentSessionId>) {
        self.current = current;
        self.parents.clear();
        self.children.clear();
        self.loaded = false;
        self.next_cursor = None;
        self.error = None;
    }

    /// The rows shown, in order: each session the filter admits, by
    /// itself or by a child it admits, followed by its children it admits.
    pub(crate) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for parent in &self.parents {
            let children: Vec<&AgentSessionSummary> = self
                .children
                .get(&parent.session_id)
                .into_iter()
                .flatten()
                .filter(|child| self.filter.admits(SessionStatus::of(child)))
                .collect();
            if !self.filter.admits(SessionStatus::of(parent)) && children.is_empty() {
                continue;
            }
            rows.push(Row {
                summary: parent.clone(),
                child: false,
            });
            rows.extend(children.into_iter().map(|child| Row {
                summary: child.clone(),
                child: true,
            }));
        }
        rows
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<CenterAction> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.notice = None;
        let rows = self.rows();
        let selected = rows.get(self.selected).map(|row| row.summary.clone());
        if let Some(session_id) = self.confirm_delete.take() {
            return matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'))
                .then_some(CenterAction::Delete(session_id));
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Some(CenterAction::Close),
            KeyCode::Char('c') if ctrl => return Some(CenterAction::Close),
            KeyCode::Tab | KeyCode::Right => self.set_filter(self.filter.step(true)),
            KeyCode::BackTab | KeyCode::Left => self.set_filter(self.filter.step(false)),
            KeyCode::Char(digit @ '1'..='5') => {
                let index = usize::from(digit as u8 - b'1');
                self.set_filter(CenterFilter::ALL[index]);
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_by(&rows, -1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(&rows, 1),
            KeyCode::PageUp => self.move_by(&rows, -self.page_step()),
            KeyCode::PageDown => self.move_by(&rows, self.page_step()),
            KeyCode::Home => self.move_by(&rows, -isize::MAX / 2),
            KeyCode::End => self.move_by(&rows, isize::MAX / 2),
            KeyCode::Char('n') => return Some(CenterAction::New),
            KeyCode::Enter if self.filter == CenterFilter::Archived => {
                return selected.map(|summary| CenterAction::SetArchived {
                    session_id: summary.session_id,
                    archive: false,
                })
            }
            KeyCode::Enter => return selected.map(CenterAction::Open),
            KeyCode::Char('f') => {
                return selected.map(|summary| CenterAction::Fork(summary.session_id))
            }
            KeyCode::Char('a') => {
                return selected.map(|summary| CenterAction::SetArchived {
                    session_id: summary.session_id,
                    archive: self.filter != CenterFilter::Archived,
                })
            }
            KeyCode::Char('d') => {
                if let Some(summary) = selected {
                    self.confirm_delete = Some(summary.session_id);
                }
            }
            KeyCode::Char('i') => match selected {
                Some(summary) if SessionStatus::of(&summary) == SessionStatus::Running => {
                    return Some(CenterAction::Interrupt(summary))
                }
                Some(_) => self.notice = Some("Nothing runs there.".to_string()),
                None => {}
            },
            _ => {}
        }
        None
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
            let shown = self.rows().len();
            let remaining = shown.saturating_sub(self.selected + 1);
            let unfilled = shown < self.view_rows.unwrap_or(PAGE_SIZE);
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
        self.parents.extend(
            page.sessions
                .into_iter()
                .filter(|session| session.latest_turn_phase.is_some()),
        );
    }

    /// The listed sessions whose children are not read yet, with the
    /// request that reads them.
    pub(crate) fn children_wanted(&self) -> Vec<(AgentSessionId, AgentSessionListRequest)> {
        self.parents
            .iter()
            .filter(|parent| !self.children.contains_key(&parent.session_id))
            .map(|parent| {
                let request = AgentSessionListRequest {
                    page_size: CHILDREN_PAGE,
                    lifecycle: self.filter.lifecycle(),
                    parent: AgentSessionParentFilter::ChildrenOf(parent.session_id),
                    ..AgentSessionListRequest::default()
                };
                (parent.session_id, request)
            })
            .collect()
    }

    /// `parent`'s children, oldest first; a failure lists none.
    pub(crate) fn ingest_children(
        &mut self,
        parent: AgentSessionId,
        page: Result<AgentSessionPage, String>,
    ) {
        let mut children = page.map(|page| page.sessions).unwrap_or_default();
        children.reverse();
        self.children.insert(parent, children);
    }

    fn set_filter(&mut self, filter: CenterFilter) {
        if filter == self.filter {
            return;
        }
        let refetch = filter.lifecycle() != self.filter.lifecycle();
        self.filter = filter;
        self.selected = 0;
        self.scroll_top = 0;
        if refetch {
            self.reload(self.current);
        }
    }

    fn page_step(&self) -> isize {
        isize::try_from(self.view_rows.unwrap_or(10).max(1)).unwrap_or(10)
    }

    fn move_by(&mut self, rows: &[Row], delta: isize) {
        if rows.is_empty() {
            return;
        }
        let last = rows.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.ensure_selected_visible(rows.len());
    }

    fn ensure_selected_visible(&mut self, count: usize) {
        let capacity = self.view_rows.unwrap_or(count).max(1);
        if self.selected < self.scroll_top {
            self.scroll_top = self.selected;
        } else if self.selected >= self.scroll_top + capacity {
            self.scroll_top = self.selected + 1 - capacity;
        }
        self.scroll_top = self.scroll_top.min(count.saturating_sub(capacity));
    }
}
