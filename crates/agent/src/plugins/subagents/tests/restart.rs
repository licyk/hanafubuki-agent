//! Shutdown captures a waiting run even when only its unread update remains.

use super::wake::{held, queued, spawn_on_second, waking};
use super::*;
use crate::test_support::reopen_service_waking;
use hanafubuki_agent_kernel::{AgentSubagentWake, AgentWakeFilter, AgentWakeSource};

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_subtree_is_lost_once_without_waking_during_restart() {
    let branch = FakeServer::start(vec![
        (spawn_on_second("leaf", "Wait."), Duration::ZERO),
        (assistant_events("Waiting for the leaf."), Duration::ZERO),
    ])
    .await;
    let leaf = FakeServer::start(vec![(held(assistant_events("Done.")), Duration::ZERO)]).await;
    let (service, paths, config, events) = waking("lost-waiting", &branch, &leaf, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let agents = subagents(&service).await;
    let child = agents
        .spawn(root, spawn("branch", "Delegate."))
        .await
        .unwrap()
        .id;
    tokio::time::timeout(crate::test_support::TEST_WAIT, async {
        loop {
            if agents.agents(root).await.unwrap().iter().any(|agent| {
                agent.id == child && matches!(agent.status, AgentRunStatus::Waiting(_))
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    service.wakes.pauses.interrupted(root);
    leaf.hold.notify_one();
    queued(&service, child, 1).await;
    wait_until("all kernel work ended", || {
        service.core.background_work(root).is_empty()
    })
    .await;
    assert_eq!(
        service.background_tasks(root).await.unwrap().agents[0].status,
        AgentSubagentState::Waiting
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    drop(agents);
    drop(service);
    for pass in 0..2 {
        let service = reopen_service_waking(
            config.clone(),
            test_host(&paths),
            &paths,
            Arc::new(events.clone()),
        );
        service.start_plugins().await;
        service.core.activate_session(root).unwrap();
        let queue = service.core.test_wake_queue(root).unwrap();
        if pass == 0 {
            assert_eq!(queue.len(), 1, "{queue:?}");
            assert!(
                matches!(&queue[0].source, AgentWakeSource::Subagent {session_id,event:AgentSubagentWake::Lost,..} if *session_id == child)
            );
            service
                .core
                .take_wake_events(root, AgentWakeFilter::SubagentStatuses)
                .await
                .unwrap();
        } else {
            assert!(
                queue.is_empty(),
                "the old child's unread update is no new loss: {queue:?}"
            );
        }
        assert_eq!(events.woken(root), 0);
        assert_eq!(branch.requests().len(), 2);
        assert_eq!(leaf.requests().len(), 1);
        assert!(service
            .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
            .is_empty());
    }
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
