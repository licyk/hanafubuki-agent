//! The wake queue is persisted with the session's runtime: it survives a
//! reopen (the agent's tests deliver it after one), and an event that does
//! not decode is dropped without the rest or the session.

use hanafubuki_agent_kernel::{
    AgentSubagentWake, AgentWakePolicy, AgentWakeQueued, AgentWakeSource,
    PersistedAgentRuntimeState,
};

use super::*;

fn lost(id: u64) -> AgentWakeSource {
    AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(id),
        name: format!("agent_{id}"),
        event: AgentSubagentWake::Lost,
    }
}

fn answering(answer: &str) -> Arc<ScriptedPersistenceModel> {
    Arc::new(ScriptedPersistenceModel {
        responses: Mutex::new(VecDeque::from([final_answer_events(answer)
            .into_iter()
            .collect::<Vec<_>>()])),
        requests: AtomicUsize::new(0),
    })
}

#[tokio::test]
async fn the_queue_survives_a_reopen() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        answering("unused"),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = service.create_session(text_session_config()).unwrap();
    service
        .set_session_wake_policy(session.id, AgentWakePolicy::On)
        .await
        .unwrap();
    for id in [41, 42] {
        assert_eq!(
            service.enqueue_wake_event(session.id, lost(id)).await,
            Ok(AgentWakeQueued::Waiting)
        );
    }
    let persisted = store.load_runtime_state(session.id).unwrap();
    assert_eq!(persisted.wake_queue.len(), 2);
    // Queuing commits no revision.
    assert_eq!(persisted.revision, session.revision);
    drop(service);

    let reopened = AgentService::with_history_store(
        answering("read them"),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    reopened.activate_session(session.id).unwrap();
    let snapshot = reopened.snapshot(session.id).unwrap();
    assert_eq!(snapshot.pending_wake_events, 2);
    // The policy is given again at each activation.
    assert_eq!(snapshot.wake_policy, AgentWakePolicy::Off);
    let queue = store.load_runtime_state(session.id).unwrap().wake_queue;
    assert_eq!(
        queue
            .events
            .into_iter()
            .map(|event| (event.sequence, event.source))
            .collect::<Vec<_>>(),
        vec![(1, lost(41)), (2, lost(42))]
    );
    // A new event continues the session's sequence.
    reopened
        .set_session_wake_policy(session.id, AgentWakePolicy::On)
        .await
        .unwrap();
    reopened
        .enqueue_wake_event(session.id, lost(43))
        .await
        .unwrap();
    let queue = store.load_runtime_state(session.id).unwrap().wake_queue;
    assert_eq!(queue.events.last().unwrap().sequence, 3);
}

/// A runtime record whose queue holds an event this build cannot read
/// (another kind, a field of the wrong type) loads with the rest.
#[test]
fn an_event_that_does_not_decode_is_dropped_and_the_session_recovers() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        answering("unused"),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = service.create_session(text_session_config()).unwrap();
    drop(service);
    let mut runtime = serde_json::to_value(store.load_runtime_state(session.id).unwrap()).unwrap();
    runtime["revision"] = json!(session.revision.value() + 1);
    runtime["wakeQueue"] = json!({
        "events": [
            { "sequence": 1, "queuedAt": 5, "source": { "type": "web_hook" } },
            {
                "sequence": 2,
                "queuedAt": 5,
                "source": { "type": "subagent", "sessionId": 41, "name": "agent_41", "event": { "type": "lost" } },
            },
            { "sequence": 3, "queuedAt": "later", "source": { "type": "subagent" } },
        ],
        "lastSequence": 3,
    });
    let runtime: PersistedAgentRuntimeState = serde_json::from_value(runtime).unwrap();
    assert_eq!(runtime.wake_queue.len(), 1);
    store
        .append_commit(AgentConversationCommit {
            runtime,
            history_mutations: Vec::new(),
        })
        .unwrap();

    let reopened = AgentService::with_history_store(
        answering("unused"),
        AgentKernelRegistries::default(),
        store,
    );
    reopened.activate_session(session.id).unwrap();
    assert_eq!(
        reopened.snapshot(session.id).unwrap().pending_wake_events,
        1
    );
}
