use super::*;

const CHILD_ROOT: &str = "HANAFUBUKI_HISTORY_CONCURRENT_ROOT";

#[test]
fn child_opens_shared_history() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        return;
    };
    let paths = AgentHistoryPaths::under(root.into());
    let workers = BackgroundWorkerGroup::default();
    let store = FileAgentHistoryStore::open(paths, workers.clone()).unwrap();
    let owned = AgentSessionId::new(1);
    assert!(store
        .load_runtime_state(owned)
        .unwrap_err()
        .to_string()
        .contains("in use by another Agent"));
    assert!(store.load_recovery_candidates().unwrap().is_empty());
    let id = store.allocate_session_id().unwrap();
    assert_ne!(id, owned);
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(id, 0, 1),
            history_mutations: vec![],
        })
        .unwrap();
    assert_eq!(
        store
            .list_sessions_page(AgentSessionListRequest::default())
            .unwrap()
            .sessions
            .len(),
        2
    );
    assert!(store
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn processes_share_history_without_recovering_or_writing_each_others_sessions() {
    let (_directory, paths, workers, store) = open_store();
    let id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(id, 0, 1),
            history_mutations: vec![],
        })
        .unwrap();
    // An unfinished checkpoint must not be taken over by the new process.
    store
        .connection()
        .unwrap()
        .execute(
            "UPDATE agent_sessions SET recovery_needed=1 WHERE session_id=?",
            [id.value() as i64],
        )
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "store::tests::multiprocess::child_opens_shared_history",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD_ROOT, paths.root.parent().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(id, 1, 1),
            history_mutations: vec![],
        })
        .unwrap();
    assert_eq!(
        store
            .list_sessions_page(AgentSessionListRequest::default())
            .unwrap()
            .sessions
            .len(),
        2
    );
    let other_workers = BackgroundWorkerGroup::default();
    assert!(FileAgentHistoryStore::open_rebuilding_index(
        paths.clone(),
        other_workers.clone(),
        Arc::new(crate::NoopAgentHistoryRefusalSink),
    )
    .is_err());
    assert!(store
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    let reopened = FileAgentHistoryStore::open(paths, other_workers.clone()).unwrap();
    assert_eq!(reopened.load_runtime_state(id).unwrap().revision.value(), 1);
    assert!(other_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
