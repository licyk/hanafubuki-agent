//! Queued appends: a follow-up an acknowledgement hands back is appended
//! next, and fails with the append when the store cannot make it.

use std::sync::mpsc;

use hanafubuki_agent_kernel::{
    AgentHistoryAppendAck, AgentHistoryFollowUp, AgentHistoryStoreError,
};

use super::*;

fn checkpoint(session_id: AgentSessionId, revision: u64) -> AgentConversationCommit {
    AgentConversationCommit {
        runtime: runtime(session_id, revision, 1),
        history_mutations: Vec::new(),
    }
}

/// An acknowledgement that reports its outcome on `outcomes` under `name`
/// and hands back `follow_up`.
fn reporting_ack(
    name: &'static str,
    outcomes: &mpsc::Sender<(&'static str, Result<(), AgentHistoryStoreError>)>,
    follow_up: Option<AgentHistoryFollowUp>,
) -> AgentHistoryAppendAck {
    let outcomes = outcomes.clone();
    AgentHistoryAppendAck::new(move |outcome| {
        let _ = outcomes.send((name, outcome));
        follow_up
    })
}

#[test]
fn a_follow_up_is_appended_before_the_next_queued_append() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store.create_session(checkpoint(session_id, 0)).unwrap();
    let (outcomes, received) = mpsc::channel();

    let follow_up = AgentHistoryFollowUp {
        commit: checkpoint(session_id, 2),
        ack: reporting_ack("follow-up", &outcomes, None),
    };
    store.enqueue_commit(
        checkpoint(session_id, 1),
        reporting_ack("first", &outcomes, Some(follow_up)),
    );
    // Queued before the writer acknowledges the first: made after the
    // follow-up, or the follow-up's older revision would be refused.
    store.enqueue_commit(
        checkpoint(session_id, 3),
        reporting_ack("next", &outcomes, None),
    );

    let order = (0..3)
        .map(|_| received.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![("first", Ok(())), ("follow-up", Ok(())), ("next", Ok(()))]
    );
    assert_eq!(
        store.load_runtime_state(session_id).unwrap().revision,
        AgentRevision::new(3)
    );
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn a_follow_up_fails_with_an_append_the_store_cannot_make() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store.create_session(checkpoint(session_id, 0)).unwrap();
    assert!(store
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5))
        .is_empty());
    let (outcomes, received) = mpsc::channel();

    let follow_up = AgentHistoryFollowUp {
        commit: checkpoint(session_id, 2),
        ack: reporting_ack("follow-up", &outcomes, None),
    };
    store.enqueue_commit(
        checkpoint(session_id, 1),
        reporting_ack("first", &outcomes, Some(follow_up)),
    );

    let order = (0..2)
        .map(|_| received.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![
            ("first", Err(AgentHistoryStoreError::ShuttingDown)),
            ("follow-up", Err(AgentHistoryStoreError::ShuttingDown)),
        ]
    );
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn a_writer_that_panicked_is_replaced_by_the_next_write() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store.create_session(checkpoint(session_id, 0)).unwrap();

    store.test_panic_next_append();
    assert!(store.append_commit(checkpoint(session_id, 1)).is_err());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !store.test_writer_exited(session_id) {
        assert!(Instant::now() < deadline, "the writer never ended");
        std::thread::sleep(Duration::from_millis(5));
    }

    store.append_commit(checkpoint(session_id, 1)).unwrap();
    assert_eq!(
        store.load_runtime_state(session_id).unwrap().revision,
        AgentRevision::new(1)
    );
    let diagnostics = workers.shutdown(Instant::now() + Duration::from_secs(2));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message
            == "worker panicked: injected Agent history writer panic"));
}

#[test]
fn shutdown_reports_a_writer_that_does_not_stop_by_its_deadline() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store.create_session(checkpoint(session_id, 0)).unwrap();
    let (entered, acknowledged) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    // The acknowledgement runs on the writer and holds it, as a stalled
    // disk would.
    store.enqueue_commit(
        checkpoint(session_id, 1),
        AgentHistoryAppendAck::new(move |outcome| {
            let _ = entered.send(outcome);
            let _ = released.recv();
            None
        }),
    );
    assert_eq!(
        acknowledged.recv_timeout(Duration::from_secs(2)).unwrap(),
        Ok(())
    );

    let started = Instant::now();
    let errors = store.shutdown(started + Duration::from_millis(200));

    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        errors,
        vec![AgentHistoryStoreError::WriterDidNotStop { session_id }]
    );
    // What the writer acknowledged was durable before it said so.
    assert_eq!(
        store.load_runtime_state(session_id).unwrap().revision,
        AgentRevision::new(1)
    );
    drop(release);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
