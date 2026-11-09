//! The hierarchical compression state's chain across a commit that fails.

use hanafubuki_agent_kernel::{
    AgentCompactionStrategy, AgentCompressionState, AgentCompressionUpdate, AgentSessionMode,
};

use super::*;

/// A state of `count` numbered messages.
fn numbered(count: usize) -> AgentCompressionState {
    let mut state = AgentCompressionState::default();
    for index in 1..=count {
        let (id, r#ref) = (format!("1:{index}"), format!("m{index:05}"));
        state.message_refs.by_raw.insert(id.clone(), r#ref.clone());
        state.message_refs.by_ref.insert(r#ref, id);
    }
    state
}

/// A commit that fails is rolled back with the chain's head, so the next
/// commit carries the change again as the delta after the last durable
/// link, and the store folds the chain to the live state.
#[tokio::test]
async fn a_rolled_back_commit_leaves_the_compression_chain_whole() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            compaction_strategy: AgentCompactionStrategy::Hierarchical,
            ..text_session_config()
        })
        .unwrap();
    let set_mode = |mode| {
        let snapshot = service.snapshot(session.id).unwrap();
        service.set_session_mode(session.id, snapshot.generation, snapshot.revision, mode)
    };
    service
        .test_set_compression_state(session.id, Some(numbered(200)))
        .unwrap();
    set_mode(AgentSessionMode::Plan).unwrap();
    let grown = numbered(201);
    service
        .test_set_compression_state(session.id, Some(grown.clone()))
        .unwrap();

    store.fail_next_append.store(true, Ordering::SeqCst);
    assert!(set_mode(AgentSessionMode::Default).is_err());
    set_mode(AgentSessionMode::Default).unwrap();

    let links = store
        .commits
        .lock()
        .unwrap()
        .iter()
        .filter_map(|commit| commit.runtime.compression_update.clone())
        .collect::<Vec<_>>();
    assert!(matches!(links[0], AgentCompressionUpdate::Snapshot(_)));
    let AgentCompressionUpdate::Delta(delta) = &links[1] else {
        panic!("a delta follows the snapshot: {links:?}");
    };
    assert_eq!(delta.seq, 1);
    assert_eq!(delta.refs.len(), 1);
    assert_eq!(
        store
            .inner
            .load_runtime_state(session.id)
            .unwrap()
            .compression,
        Some(grown)
    );
}
