use std::fs;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

use hanafubuki_agent::AgentSessionId;
use serde_json::{json, Value};

use super::{TuiState, MAX_SEEN, TUI_STATE_FILE};
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};

fn load(dir: &Path) -> (TuiState, Option<String>, Receiver<AppEvent>) {
    let (sender, events) = channel();
    let (state, warning) = TuiState::load(dir, AppEventSender::new(sender));
    (state, warning, events)
}

fn file(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join(TUI_STATE_FILE)).unwrap()).unwrap()
}

fn recovered(dir: &Path) -> Vec<String> {
    fs::read_dir(dir.join(".recovery"))
        .map(|entries| {
            entries
                .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
                .collect()
        })
        .unwrap_or_default()
}

fn id(value: u64) -> AgentSessionId {
    AgentSessionId::new(value)
}

#[test]
fn the_marks_are_written_and_read_back() {
    let home = tempfile::tempdir().unwrap();
    let (mut state, warning, _events) = load(home.path());
    assert_eq!(warning, None, "no file is no problem");
    state.agents.pinned.insert(id(3));
    state.agents.hidden.insert(id(7));
    state.agents.seen.insert(id(3), 1_700_000_000_000);
    state.save_now().unwrap();
    assert_eq!(
        file(home.path()),
        json!({ "agents": {
            "pinned": [3],
            "hidden": [7],
            "seen": { "3": 1_700_000_000_000_u64 },
        }})
    );
    let (again, warning, _events) = load(home.path());
    assert_eq!(warning, None);
    assert_eq!(again.agents, state.agents);
    assert!(again.agents.unread(id(3), 1_700_000_000_001));
    assert!(!again.agents.unread(id(3), 1_700_000_000_000));
    assert!(!again.agents.unread(id(4), 1), "a session never shown");
}

#[test]
fn a_save_off_the_loop_writes_the_latest_state() {
    let home = tempfile::tempdir().unwrap();
    let (mut state, _, _events) = load(home.path());
    for value in 1..=5 {
        state.agents.pinned.insert(id(value));
        state.save();
    }
    // The writes are serialized; wait for the last to land. A read may
    // meet the file between two writes' renames, so it is read tolerantly.
    let path = home.path().join(TUI_STATE_FILE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let written = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if written.is_some_and(|state| state["agents"]["pinned"] == json!([1, 2, 3, 4, 5])) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the last write never landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn unknown_keys_are_kept_as_they_were() {
    let home = tempfile::tempdir().unwrap();
    let original = json!({
        "later": { "a": 1 },
        "agents": { "pinned": [1], "sections": ["x"] },
    });
    fs::write(home.path().join(TUI_STATE_FILE), original.to_string()).unwrap();
    let (mut state, warning, _events) = load(home.path());
    assert_eq!(warning, None);
    state.agents.hidden.insert(id(2));
    state.save_now().unwrap();
    let written = file(home.path());
    assert_eq!(written["later"], json!({ "a": 1 }));
    assert_eq!(written["agents"]["sections"], json!(["x"]));
    assert_eq!(written["agents"]["hidden"], json!([2]));
    assert!(recovered(home.path()).is_empty(), "nothing was left out");
}

#[test]
fn an_entry_not_understood_is_left_out_and_the_file_moved_aside_before_a_write() {
    let home = tempfile::tempdir().unwrap();
    let original = json!({ "agents": {
        "pinned": [1, "two", 3],
        "hidden": "all",
        "seen": { "4": 10, "five": 11, "6": "soon" },
    }});
    fs::write(home.path().join(TUI_STATE_FILE), original.to_string()).unwrap();
    let (mut state, warning, _events) = load(home.path());
    assert_eq!(warning, None);
    assert_eq!(
        state.agents.pinned.iter().copied().collect::<Vec<_>>(),
        [id(1), id(3)]
    );
    assert!(state.agents.hidden.is_empty());
    assert_eq!(state.agents.seen.len(), 1);
    // Read alone, the file is left as it is.
    assert!(recovered(home.path()).is_empty());
    state.save_now().unwrap();
    let kept = recovered(home.path());
    assert_eq!(kept.len(), 1);
    assert_eq!(serde_json::from_str::<Value>(&kept[0]).unwrap(), original);
    assert_eq!(file(home.path())["agents"]["pinned"], json!([1, 3]));
}

#[test]
fn a_file_that_is_not_a_json_object_is_moved_aside_at_once() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join(TUI_STATE_FILE), "[1, 2").unwrap();
    let (mut state, warning, _events) = load(home.path());
    assert!(warning.unwrap().contains("moved aside"));
    assert_eq!(recovered(home.path()), ["[1, 2"]);
    assert_eq!(state.agents, Default::default());
    state.save_now().unwrap();
}

#[test]
fn a_file_that_cannot_be_read_is_never_written_over() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(TUI_STATE_FILE)).unwrap();
    let (mut state, warning, events) = load(home.path());
    assert!(warning.unwrap().contains("was not read"));
    state.agents.pinned.insert(id(1));
    assert!(state.save_now().is_err());
    state.save();
    assert!(matches!(
        events.recv().unwrap(),
        AppEvent::Workspace(WorkspaceEvent::TuiStateNotSaved(_))
    ));
    assert!(home.path().join(TUI_STATE_FILE).is_dir());
}

#[test]
fn only_the_latest_showings_are_kept() {
    let home = tempfile::tempdir().unwrap();
    let (mut state, _, _events) = load(home.path());
    let count = u64::try_from(MAX_SEEN).unwrap() + 10;
    for value in 0..count {
        state.agents.seen.insert(id(value), u128::from(value));
    }
    state.save_now().unwrap();
    let seen = &state.agents.seen;
    assert_eq!(seen.len(), MAX_SEEN);
    assert!(!seen.contains_key(&id(9)) && seen.contains_key(&id(10)));
}
