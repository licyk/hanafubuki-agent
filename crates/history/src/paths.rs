use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, Timelike};
use hanafubuki_agent_foundation::paths::PathKey;
use hanafubuki_agent_kernel::{AgentHistoryStoreError, AgentSessionId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHistoryPaths {
    pub root: PathBuf,
    pub active_sessions: PathBuf,
    pub archived_sessions: PathBuf,
    pub trash: PathBuf,
    pub attachments: PathBuf,
    pub sqlite_database: PathBuf,
}

impl AgentHistoryPaths {
    pub fn under(agent_root: PathBuf) -> Self {
        let root = agent_root.join("history");
        Self {
            active_sessions: root.join("active"),
            archived_sessions: root.join("archived"),
            trash: root.join("trash"),
            attachments: root.join("attachments"),
            sqlite_database: root.join("index.sqlite3"),
            root,
        }
    }
}

pub(crate) fn new_session_path(paths: &AgentHistoryPaths, session_id: AgentSessionId) -> PathBuf {
    let now = Local::now();
    let directory = paths
        .active_sessions
        .join(format!("{:04}", now.year()))
        .join(format!("{:02}", now.month()))
        .join(format!("{:02}", now.day()));
    let timestamp = format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    directory.join(format!("session-{timestamp}-{session_id}.jsonl"))
}

/// Reject a persisted history path that no longer lies under the store root.
///
/// The path comes from a database row and the root from the current layout, so
/// the two need not agree on spelling; identity is what the check is about.
pub(crate) fn ensure_scoped_path(path: &Path, root: &Path) -> Result<(), AgentHistoryStoreError> {
    if !PathKey::of(path).starts_with(&PathKey::of(root)) {
        return Err(AgentHistoryStoreError::CorruptHistory(format!(
            "history path {} escapes {}",
            path.display(),
            root.display()
        )));
    }
    Ok(())
}
