//! What a parent hears of its agents through its wake queue: a final
//! status is queued on it durably, `wait_agent` takes it once (after a
//! restart too), and a follow-up task withdraws the status its previous
//! turn left.

use super::*;
use crate::test_support::reopen_service;

/// Waits until `session_id` has `count` wake events queued.
async fn queued(service: &AgentApplicationService, session_id: AgentSessionId, count: usize) {
    wait_until(&format!("{count} wake events are queued"), || {
        service.snapshot(session_id).unwrap().pending_wake_events == count
    })
    .await;
}

/// An agent's final status is queued on its parent, which takes it with
/// its next wait, after a restart too, and only once.
#[tokio::test(flavor = "multi_thread")]
async fn a_final_status_waits_in_the_parents_queue_across_a_restart() {
    let server = FakeServer::start(vec![(assistant_events("Twelve files."), Duration::ZERO)]).await;
    let (service, paths, config) = service_with_host("subagents-queued", test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    service.start_plugins().await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let child = subagents
        .spawn(parent, spawn("counter", "Count the files."))
        .await
        .unwrap()
        .id;
    queued(&service, parent, 1).await;
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let reopened = reopen_service(config, test_host(&paths), &paths);
    reopened.start_plugins().await;
    let subagents = super::subagents(&reopened).await;
    let cancellation = CancellationToken::default();
    assert_eq!(
        subagents
            .wait(parent, NO_WAIT, &cancellation)
            .await
            .unwrap(),
        WaitOutcome::Updates(vec![AgentUpdate {
            name: "counter".to_string(),
            id: child,
            status: AgentRunStatus::Completed(Some("Twelve files.".to_string())),
        }])
    );
    assert_eq!(reopened.snapshot(parent).unwrap().pending_wake_events, 0);
    assert_eq!(
        subagents
            .wait(parent, NO_WAIT, &cancellation)
            .await
            .unwrap(),
        WaitOutcome::Idle,
        "a status is reported once"
    );
    finish(reopened, paths);
}

/// A follow-up task given to an agent whose last status was not waited for
/// withdraws it: the next wait reports the follow-up's end, as it did when
/// the status was kept in memory.
#[tokio::test(flavor = "multi_thread")]
async fn a_follow_up_withdraws_the_status_its_previous_turn_left() {
    let server = FakeServer::start(vec![
        (assistant_events("First answer."), Duration::ZERO),
        (assistant_events("Second answer."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = started("subagents-withdrawn", &server).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    subagents
        .spawn(parent, spawn("worker", "Answer once."))
        .await
        .unwrap();
    queued(&service, parent, 1).await;

    assert_eq!(
        subagents
            .followup_task(parent, "worker", "Answer again.".into())
            .await
            .unwrap(),
        Delivery::Started
    );
    let waited = subagents
        .wait(parent, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap();
    assert!(
        matches!(
            &waited,
            WaitOutcome::Updates(updates)
                if updates.len() == 1
                    && updates[0].status
                        == AgentRunStatus::Completed(Some("Second answer.".to_string()))
        ),
        "{waited:?}"
    );
    finish(service, paths);
}
