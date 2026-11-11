//! Sessions the history keeps but cannot open, as the running Agent meets
//! them: one the session list refuses joins the status's quarantine at
//! once, and a session one of them forks from is not deleted.

use hanafubuki_agent_kernel::{AgentSessionId, AgentSessionListRequest};

use super::startup::{marker, start, stop, stop_runtime, unrecovered, CountingStatusSink};
use super::*;

fn history_paths(paths: &TestPaths) -> hanafubuki_agent_history::AgentHistoryPaths {
    hanafubuki_agent_history::AgentHistoryPaths::under(
        AgentStoragePaths::under(paths.data_dir.join("agent-test")).history_root,
    )
}

fn rollout_of(root: &std::path::Path, session_id: AgentSessionId) -> std::path::PathBuf {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(&format!("-{session_id}.jsonl"))
            {
                return path;
            }
        }
    }
    panic!("no rollout of session {session_id}");
}

#[test]
fn a_session_the_list_refuses_joins_the_agent_status_at_once() {
    let (service, paths) = configured_service("quarantine-list-refusal", "http://127.0.0.1:1/v1");
    let setup = tokio::runtime::Runtime::new().unwrap();
    let [broken, intact] = [0, 1].map(|_| {
        setup
            .block_on(service.create_session(AgentSessionMode::Default, None))
            .unwrap()
            .id
    });
    drop(setup);
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    drop(service);
    let storage = AgentStoragePaths::under(paths.data_dir.join("agent-test"));
    let events = Arc::new(CountingStatusSink::default());
    let slot = AgentSlot::new(None, events.clone());
    let agent = start(&slot, &marker(&paths), unrecovered(&paths, storage)).unwrap();
    assert!(slot.status().quarantined_sessions.is_empty());

    rusqlite::Connection::open(history_paths(&paths).sqlite_database)
        .unwrap()
        .execute(
            "UPDATE agent_sessions SET summary_json = '{broken' WHERE session_id = ?",
            [i64::try_from(broken.value()).unwrap()],
        )
        .unwrap();
    let before = events.0.load(Ordering::SeqCst);
    let listed = || {
        agent
            .list_sessions_page(AgentSessionListRequest::default())
            .unwrap()
            .sessions
            .iter()
            .map(|summary| summary.session_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(listed(), vec![intact]);

    let status = slot.status();
    assert_eq!(status.quarantined_sessions.len(), 1);
    assert_eq!(status.quarantined_sessions[0].session_id, broken);
    assert!(status.quarantined_sessions[0].reason.contains("summary"));
    assert_eq!(events.0.load(Ordering::SeqCst), before + 1);
    // Listed again, it is neither added nor announced again.
    assert_eq!(listed(), vec![intact]);
    assert_eq!(slot.status().quarantined_sessions.len(), 1);
    assert_eq!(events.0.load(Ordering::SeqCst), before + 1);
    assert!(agent.open_session(broken, 75).is_err());
    stop(agent);
    stop_runtime(&slot);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The refusal comes from the deletion's prepare step, before the session's
/// memory is revoked (which signals a memory scan).
#[test]
fn a_parent_a_quarantined_fork_needs_is_not_deleted_and_keeps_its_memory() {
    let (service, paths) =
        configured_service("quarantine-parent-deletion", "http://127.0.0.1:1/v1");
    let setup = tokio::runtime::Runtime::new().unwrap();
    let parent = setup
        .block_on(service.create_session(AgentSessionMode::Default, None))
        .unwrap();
    drop(setup);
    let child = service
        .fork_session_at_head(hanafubuki_agent_kernel::AgentSessionHeadForkRequest {
            source_session_id: parent.id,
            source_generation: parent.generation,
            source_revision: parent.revision,
        })
        .unwrap()
        .session
        .id;
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    drop(service);
    let history = history_paths(&paths);
    let child_rollout = rollout_of(&history.active_sessions, child);
    let another_format = std::fs::read_to_string(&child_rollout)
        .unwrap()
        .lines()
        .map(|line| {
            let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
            record["formatVersion"] =
                json!(hanafubuki_agent_kernel::AGENT_HISTORY_FORMAT_VERSION + 1);
            format!("{record}\n")
        })
        .collect::<String>();
    std::fs::write(&child_rollout, another_format).unwrap();
    std::fs::remove_file(&history.sqlite_database).unwrap();

    let storage = AgentStoragePaths::under(paths.data_dir.join("agent-test"));
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let agent = start(&slot, &marker(&paths), unrecovered(&paths, storage)).unwrap();
    assert_eq!(
        slot.status()
            .quarantined_sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        vec![child]
    );
    let opened = agent.open_session(parent.id, 75).unwrap().runtime;
    let signals = agent.test_memory().test_scan_signals();
    let error = agent
        .delete_session_confirmed(parent.id, opened.generation, opened.revision)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&format!("Agent session {} cannot be deleted", parent.id))
            && error.contains(&format!("session {child} at ")),
        "unexpected error: {error}"
    );
    assert_eq!(agent.test_memory().test_scan_signals(), signals);
    assert!(rollout_of(&history.active_sessions, parent.id).is_file());
    let reopened = agent.open_session(parent.id, 75).unwrap().runtime;
    assert_eq!(reopened.revision, opened.revision);
    stop(agent);
    stop_runtime(&slot);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
