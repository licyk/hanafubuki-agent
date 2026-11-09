// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Session-based orchestration for `@` file searches.
//!
//! `ChatComposer` publishes every change of the `@token` as
//! `AppEvent::StartFileSearch(query)`. This manager owns a single search
//! session over the workspace roots, updates the query on every keystroke,
//! and drops the session when the query becomes empty.

mod search;

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::app_event::{AppEvent, AppEventSender};
pub(crate) use search::FileMatch;
use search::{FileSearchSession, FileSearchSnapshot, SessionReporter};

pub(crate) struct FileSearchManager {
    state: Arc<Mutex<SearchState>>,
    roots: Vec<PathBuf>,
    app_tx: AppEventSender,
}

struct SearchState {
    latest_query: String,
    session: Option<FileSearchSession>,
    session_token: usize,
}

impl FileSearchManager {
    /// Searches `roots`, the first the one mentions are relative to.
    pub(crate) fn new(roots: Vec<PathBuf>, tx: AppEventSender) -> Self {
        Self {
            state: Arc::new(Mutex::new(SearchState {
                latest_query: String::new(),
                session: None,
                session_token: 0,
            })),
            roots,
            app_tx: tx,
        }
    }

    /// Call whenever the user edits the `@` token.
    pub(crate) fn on_user_query(&self, query: String) {
        let mut st = self.state.lock();
        if query == st.latest_query {
            return;
        }
        st.latest_query.clear();
        st.latest_query.push_str(&query);

        if query.is_empty() {
            st.session.take();
            return;
        }

        if st.session.is_none() {
            self.start_session_locked(&mut st);
        }
        if let Some(session) = st.session.as_ref() {
            session.update_query(&query);
        }
    }

    fn start_session_locked(&self, st: &mut SearchState) {
        st.session_token = st.session_token.wrapping_add(1);
        let reporter = Arc::new(TuiSessionReporter {
            state: self.state.clone(),
            app_tx: self.app_tx.clone(),
            session_token: st.session_token,
        });
        st.session = Some(search::create_session(self.roots.clone(), reporter));
    }
}

struct TuiSessionReporter {
    state: Arc<Mutex<SearchState>>,
    app_tx: AppEventSender,
    session_token: usize,
}

impl TuiSessionReporter {
    fn send_snapshot(&self, snapshot: &FileSearchSnapshot) {
        let st = self.state.lock();
        if st.session_token != self.session_token
            || st.latest_query.is_empty()
            || snapshot.query.is_empty()
        {
            return;
        }
        let query = snapshot.query.clone();
        drop(st);
        self.app_tx.send(AppEvent::FileSearchResult {
            query,
            matches: snapshot.matches.clone(),
        });
    }
}

impl SessionReporter for TuiSessionReporter {
    fn on_update(&self, snapshot: &FileSearchSnapshot) {
        self.send_snapshot(snapshot);
    }

    fn on_complete(&self) {}
}
