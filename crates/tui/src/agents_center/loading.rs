//! What the center reads and when: the sessions from the first page on
//! (more as the selection nears the end), each listed session's children,
//! and the latest exchange of the one selected, each read once at a time
//! off the interface's loop. The watch's view of the sessions the Agent
//! holds says which changed: a changed session without a parent reads the
//! list again, a changed child its parent's children, and a session whose
//! revision moved its exchange. A new search or filter starts a new epoch,
//! whose reads leave the older ones' answers unused; a read again replaces
//! what was shown, in place.

use std::collections::{HashMap, HashSet};

use hanafubuki_agent::{AgentSessionId, AgentSessionListRequest, AgentSessionParentFilter};

use super::grouping::Place;
use super::AgentsCenter;
use crate::agent_bridge::{CenterJob, CenterRead, LiveSession, RootsRead};

pub(super) const PAGE_SIZE: usize = 25;
/// How many of a session's children are read.
const CHILDREN_PAGE: usize = 50;
const LOAD_NEAR_THRESHOLD: usize = 5;

#[derive(Debug, Default)]
pub(super) struct Reads {
    epoch: u64,
    /// The list was read in this epoch.
    pub(super) loaded: bool,
    /// More sessions follow those read.
    more: bool,
    /// How many sessions the reads of the list read.
    read: usize,
    roots_reading: bool,
    roots_stale: bool,
    children_reading: HashSet<AgentSessionId>,
    children_stale: HashSet<AgentSessionId>,
    exchange_reading: Option<AgentSessionId>,
    /// The watch was heard from once: later views are changes.
    watched: bool,
}

impl AgentsCenter {
    /// The reads the center needs now, each marked as under way.
    pub(crate) fn jobs(&mut self) -> Vec<CenterJob> {
        let mut jobs = Vec::new();
        if let Some(wanted) = self.roots_wanted() {
            self.reads.roots_reading = true;
            self.reads.roots_stale = false;
            jobs.push(CenterJob::Roots {
                epoch: self.reads.epoch,
                request: self.roots_request(),
                wanted,
            });
        }
        let parents: Vec<AgentSessionId> = self
            .roots
            .iter()
            .map(|root| root.session_id)
            .filter(|id| {
                (!self.children.contains_key(id) || self.reads.children_stale.contains(id))
                    && !self.reads.children_reading.contains(id)
            })
            .collect();
        for parent in parents {
            self.reads.children_stale.remove(&parent);
            self.reads.children_reading.insert(parent);
            jobs.push(CenterJob::Children {
                epoch: self.reads.epoch,
                parent,
                request: AgentSessionListRequest {
                    page_size: CHILDREN_PAGE,
                    lifecycle: self.filter.lifecycle(),
                    parent: AgentSessionParentFilter::ChildrenOf(parent),
                    ..AgentSessionListRequest::default()
                },
            });
        }
        if let Some(session_id) = self.exchange_wanted() {
            self.reads.exchange_reading = Some(session_id);
            jobs.push(CenterJob::Exchange(session_id));
        }
        jobs
    }

    /// How many sessions the list should read now, if it should: the
    /// first page, the list again after a change, or one page more while
    /// the selection nears the end of the rows or they do not fill it.
    fn roots_wanted(&self) -> Option<usize> {
        let reads = &self.reads;
        if reads.roots_reading {
            return None;
        }
        if !reads.loaded {
            return self.error.is_none().then_some(PAGE_SIZE);
        }
        let rows = self.rows();
        let remaining = rows.len().saturating_sub(self.selected_index(&rows) + 1);
        let unfilled = rows.len() < self.view_rows.unwrap_or(PAGE_SIZE);
        if reads.more && (remaining <= LOAD_NEAR_THRESHOLD || unfilled) {
            return Some(reads.read + PAGE_SIZE);
        }
        reads.roots_stale.then_some(reads.read.max(PAGE_SIZE))
    }

    fn roots_request(&self) -> AgentSessionListRequest {
        AgentSessionListRequest {
            page_size: PAGE_SIZE,
            lifecycle: self.filter.lifecycle(),
            search: self.query().map(str::to_string),
            ..AgentSessionListRequest::default()
        }
    }

    /// The selected session's exchange, when it was not read or its
    /// revision moved since.
    fn exchange_wanted(&self) -> Option<AgentSessionId> {
        if self.reads.exchange_reading.is_some() {
            return None;
        }
        let session_id = self.selected_row()?.summary.session_id;
        let revision = self.live.get(&session_id).map(|live| live.revision);
        match self.exchanges.get(&session_id) {
            Some((read_at, _)) if revision.is_none() || *read_at == revision => None,
            _ => Some(session_id),
        }
    }

    /// What a read found, or what the watch saw.
    pub(crate) fn ingest(&mut self, read: CenterRead) {
        match read {
            CenterRead::Roots { epoch, read } if epoch == self.reads.epoch => {
                self.reads.roots_reading = false;
                self.ingest_roots(read);
            }
            CenterRead::Children {
                epoch,
                parent,
                read,
            } if epoch == self.reads.epoch => {
                self.reads.children_reading.remove(&parent);
                // A failure reads as none.
                self.children.insert(parent, read.unwrap_or_default());
            }
            CenterRead::Exchange {
                session_id,
                exchange,
            } => {
                self.reads.exchange_reading = None;
                let revision = self.live.get(&session_id).map(|live| live.revision);
                self.exchanges.insert(session_id, (revision, exchange));
            }
            CenterRead::Live(live) => self.ingest_live(live),
            CenterRead::Roots { .. } | CenterRead::Children { .. } => {}
        }
    }

    fn ingest_roots(&mut self, read: Result<RootsRead, String>) {
        let read = match read {
            Ok(read) => read,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.error = None;
        self.reads.loaded = true;
        self.reads.more = read.more;
        self.reads.read = read.sessions.len();
        self.roots.clear();
        // A session no turn ran in has nothing to show, as in the picker.
        for session in read.sessions {
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

    /// The watch's view: what changed since the last one is read again.
    fn ingest_live(&mut self, live: Vec<LiveSession>) {
        let live: HashMap<AgentSessionId, LiveSession> = live
            .into_iter()
            .map(|session| (session.session_id, session))
            .collect();
        if self.reads.watched {
            let changed = live
                .values()
                .filter(|session| self.live.get(&session.session_id) != Some(*session))
                .chain(
                    self.live
                        .values()
                        .filter(|old| !live.contains_key(&old.session_id)),
                );
            for session in changed {
                match session.parent {
                    None => self.reads.roots_stale = true,
                    Some(parent) if self.children.contains_key(&parent) => {
                        self.reads.children_stale.insert(parent);
                    }
                    Some(_) => {}
                }
            }
        }
        self.reads.watched = true;
        self.live = live;
    }

    /// The list read anew from its first page, for another filter or
    /// search; the selection stays on its session.
    pub(super) fn reload(&mut self) {
        self.reads = Reads {
            epoch: self.reads.epoch + 1,
            watched: self.reads.watched,
            ..Reads::default()
        };
        self.roots.clear();
        self.children.clear();
        self.child_selected = None;
        self.error = None;
    }

    /// After an action: everything shown read again, in place.
    pub(crate) fn refresh(&mut self, current: Option<AgentSessionId>) {
        self.current = current;
        self.reads.roots_stale = true;
        self.reads.children_stale = self.children.keys().copied().collect();
        self.exchanges.clear();
    }
}
