//! The interface's own state in the data directory, `tui-state.json`: the
//! sessions the command center pins and hides, and when each session was
//! last shown, from which a row is marked unread (decision D11 of the
//! settings-and-timeline plan). The sessions' own records know nothing of
//! it.
//!
//! It is read tolerantly: an unknown key is kept as it was, an entry that is
//! not understood is left out, and the file is then moved aside into
//! `.recovery/` before it is next written; a file that is not a JSON object
//! is moved aside at once. A file that exists but cannot be read is never
//! written over. Writes are made off the interface's loop, the latest one
//! last.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hanafubuki_agent::{quarantine_file, write_atomic, AgentSessionId, FileAccess};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};

pub(crate) const TUI_STATE_FILE: &str = "tui-state.json";
const MAX_STATE_BYTES: u64 = 1024 * 1024;
/// The most sessions whose last showing is kept; the oldest go first.
const MAX_SEEN: usize = 2000;

/// What the command center keeps of the sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentsMarks {
    pub(crate) pinned: BTreeSet<AgentSessionId>,
    pub(crate) hidden: BTreeSet<AgentSessionId>,
    /// When each session was last shown (milliseconds since the epoch).
    pub(crate) seen: BTreeMap<AgentSessionId, u128>,
}

impl AgentsMarks {
    /// Whether `session_id` changed (`updated_at`) after it was last shown;
    /// one never shown here is not.
    pub(crate) fn unread(&self, session_id: AgentSessionId, updated_at: u128) -> bool {
        self.seen
            .get(&session_id)
            .is_some_and(|seen| updated_at > *seen)
    }

    /// Forgets a deleted session.
    pub(crate) fn forget(&mut self, session_id: AgentSessionId) {
        self.pinned.remove(&session_id);
        self.hidden.remove(&session_id);
        self.seen.remove(&session_id);
    }
}

pub(crate) struct TuiState {
    path: PathBuf,
    /// The file as read, its other keys kept as they were.
    document: Map<String, Value>,
    pub(crate) agents: AgentsMarks,
    /// An entry was not understood: the file is moved aside first.
    lossy: bool,
    /// Why the file, which exists, was not read; it is then never written.
    unreadable: Option<String>,
    written: Arc<Mutex<u64>>,
    generation: u64,
    app_event_tx: AppEventSender,
}

impl TuiState {
    /// The state in `data_dir`, and what was wrong with its file.
    pub(crate) fn load(data_dir: &Path, app_event_tx: AppEventSender) -> (Self, Option<String>) {
        let path = data_dir.join(TUI_STATE_FILE);
        let mut state = Self {
            path,
            document: Map::new(),
            agents: AgentsMarks::default(),
            lossy: false,
            unreadable: None,
            written: Arc::new(Mutex::new(0)),
            generation: 0,
            app_event_tx,
        };
        let warning = state.read();
        (state, warning)
    }

    /// Writes the state off the interface's loop; a failure comes back as
    /// [`WorkspaceEvent::TuiStateNotSaved`].
    pub(crate) fn save(&mut self) {
        let bytes = match self.prepare() {
            Ok(bytes) => bytes,
            Err(error) => return self.failed(error),
        };
        self.generation += 1;
        let generation = self.generation;
        let (path, written, app_event_tx) = (
            self.path.clone(),
            self.written.clone(),
            self.app_event_tx.clone(),
        );
        std::thread::spawn(move || {
            let mut latest = written.lock();
            // A later state already written stays.
            if *latest > generation {
                return;
            }
            *latest = generation;
            if let Err(error) = write_atomic(&path, &bytes, FileAccess::Private) {
                app_event_tx.send(not_saved(error));
            }
        });
    }

    /// Writes the state now, as the interface ends.
    pub(crate) fn save_now(&mut self) -> Result<(), String> {
        let bytes = self.prepare()?;
        self.generation += 1;
        let mut latest = self.written.lock();
        *latest = self.generation;
        write_atomic(&self.path, &bytes, FileAccess::Private)
    }

    fn failed(&self, error: String) {
        self.app_event_tx.send(not_saved(error));
    }

    /// The bytes to write, once a file not wholly understood is moved aside.
    fn prepare(&mut self) -> Result<Vec<u8>, String> {
        if let Some(reason) = &self.unreadable {
            return Err(format!(
                "{TUI_STATE_FILE} was not read, so it is not written: {reason}"
            ));
        }
        if self.lossy {
            quarantine_file(&self.path)?;
            self.lossy = false;
        }
        self.trim_seen();
        let agents = match self.document.remove("agents") {
            Some(Value::Object(mut agents)) => {
                encode(&self.agents, &mut agents);
                agents
            }
            _ => {
                let mut agents = Map::new();
                encode(&self.agents, &mut agents);
                agents
            }
        };
        self.document
            .insert("agents".to_string(), Value::Object(agents));
        serde_json::to_vec_pretty(&self.document)
            .map_err(|error| format!("{TUI_STATE_FILE} was not encoded: {error}"))
    }

    fn trim_seen(&mut self) {
        let seen = &mut self.agents.seen;
        if seen.len() <= MAX_SEEN {
            return;
        }
        let mut times: Vec<u128> = seen.values().copied().collect();
        times.sort_unstable();
        let oldest_kept = times[times.len() - MAX_SEEN];
        seen.retain(|_, time| *time >= oldest_kept);
    }

    /// Reads the file; the warning says what was set aside.
    fn read(&mut self) -> Option<String> {
        let bytes = match read_bounded(&self.path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(Unread::Unreadable(reason)) => {
                let warning = format!("{TUI_STATE_FILE} was not read: {reason}");
                self.unreadable = Some(reason);
                return Some(warning);
            }
            Err(Unread::TooLarge) => return self.set_aside("it is too large"),
        };
        let Ok(Value::Object(document)) = serde_json::from_slice(&bytes) else {
            return self.set_aside("it is not a JSON object");
        };
        let (agents, lossy) = decode(document.get("agents"));
        self.document = document;
        self.agents = agents;
        self.lossy = lossy;
        None
    }

    fn set_aside(&mut self, reason: &str) -> Option<String> {
        Some(match quarantine_file(&self.path) {
            Ok(_) => format!("{TUI_STATE_FILE} was moved aside into .recovery/: {reason}"),
            Err(error) => {
                self.unreadable = Some(error.clone());
                format!("{TUI_STATE_FILE} could not be moved aside ({reason}): {error}")
            }
        })
    }
}

fn not_saved(error: String) -> AppEvent {
    AppEvent::Workspace(WorkspaceEvent::TuiStateNotSaved(error))
}

enum Unread {
    TooLarge,
    Unreadable(String),
}

/// The file's bytes; `None` when there is no file.
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, Unread> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Unread::Unreadable(error.to_string())),
    };
    if !metadata.is_file() {
        return Err(Unread::Unreadable("it is not a file".to_string()));
    }
    if metadata.len() > MAX_STATE_BYTES {
        return Err(Unread::TooLarge);
    }
    fs::read(path)
        .map(Some)
        .map_err(|error| Unread::Unreadable(error.to_string()))
}

/// The marks in `agents`, and whether any entry was left out.
fn decode(agents: Option<&Value>) -> (AgentsMarks, bool) {
    let mut lossy = false;
    let mut marks = AgentsMarks::default();
    let Some(agents) = agents else {
        return (marks, lossy);
    };
    let Some(agents) = agents.as_object() else {
        return (marks, true);
    };
    marks.pinned = ids(agents.get("pinned"), &mut lossy);
    marks.hidden = ids(agents.get("hidden"), &mut lossy);
    match agents.get("seen") {
        None => {}
        Some(Value::Object(seen)) => {
            for (id, time) in seen {
                match (id.parse::<u64>(), time.as_u64()) {
                    (Ok(id), Some(time)) => {
                        marks.seen.insert(AgentSessionId::new(id), u128::from(time));
                    }
                    _ => lossy = true,
                }
            }
        }
        Some(_) => lossy = true,
    }
    (marks, lossy)
}

fn ids(value: Option<&Value>, lossy: &mut bool) -> BTreeSet<AgentSessionId> {
    match value {
        None => BTreeSet::new(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|value| {
                let id = value.as_u64().map(AgentSessionId::new);
                *lossy |= id.is_none();
                id
            })
            .collect(),
        Some(_) => {
            *lossy = true;
            BTreeSet::new()
        }
    }
}

fn encode(marks: &AgentsMarks, agents: &mut Map<String, Value>) {
    let ids =
        |set: &BTreeSet<AgentSessionId>| -> Vec<u64> { set.iter().map(|id| id.value()).collect() };
    agents.insert("pinned".to_string(), json!(ids(&marks.pinned)));
    agents.insert("hidden".to_string(), json!(ids(&marks.hidden)));
    let seen: Map<String, Value> = marks
        .seen
        .iter()
        .map(|(id, time)| {
            (
                id.value().to_string(),
                json!(u64::try_from(*time).unwrap_or(u64::MAX)),
            )
        })
        .collect();
    agents.insert("seen".to_string(), Value::Object(seen));
}

#[cfg(test)]
mod tests;
