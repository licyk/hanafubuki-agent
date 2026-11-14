use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentExecutionProgressStatus,
    AgentSessionMode, AgentSessionPersistence, AgentToolProgressUpdate, AgentWakeFilter,
};

use super::*;
use crate::test_support::*;
use crate::AgentConfig;

fn open(
    paths: &TestPaths,
    config: &AgentConfig,
    events: &RecordedBackgroundEvents,
) -> AgentApplicationService {
    reopen_service_waking(
        config.clone(),
        test_host(paths),
        paths,
        Arc::new(events.clone()),
    )
}

async fn setup(
    name: &str,
    server: &FakeServer,
) -> (
    AgentApplicationService,
    TestPaths,
    AgentConfig,
    RecordedBackgroundEvents,
    AgentSessionId,
) {
    let paths = test_paths(name);
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &server.base_url, None);
    config.set_plugin_disabled("titles", true).unwrap();
    let events = RecordedBackgroundEvents::default();
    let service = open(&paths, &config, &events);
    service.start_plugins().await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    (service, paths, config, events, root)
}

fn child(service: &AgentApplicationService, parent: AgentSessionId) -> AgentSessionId {
    service
        .driver
        .create_child(AgentChildSessionRequest {
            parent_session_id: parent,
            origin: AgentSessionOrigin::Subagent,
            persistence: AgentSessionPersistence::Durable,
            base: AgentChildSessionBase::Empty,
            context: Vec::new(),
            scope: Default::default(),
            role_instructions: None,
        })
        .unwrap()
        .id
}

async fn requested(server: &FakeServer, count: usize) {
    tokio::time::timeout(TEST_WAIT, async {
        while server.requests().len() < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

fn stop(service: &AgentApplicationService) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
}

fn lost_agent(service: &AgentApplicationService, owner: AgentSessionId, child: AgentSessionId) {
    service.core.activate_session(owner).unwrap();
    let queued = service.core.test_wake_queue(owner).unwrap();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(matches!(&queued[0].source,
        AgentWakeSource::Subagent { session_id, event: AgentSubagentWake::Lost, .. } if *session_id == child));
}

fn held() -> Vec<u8> {
    [HOLD.as_bytes(), &assistant_events("Too late.")].concat()
}

// Only canonical rollouts are copied. Reopening reconstructs the index as
// after a crash; no live SQLite file is copied.
fn rollouts(directory: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut saved = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            saved.extend(rollouts(&path));
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            saved.push((path.clone(), std::fs::read(path).unwrap()));
        }
    }
    saved
}

fn restore(paths: &TestPaths, saved: Vec<(PathBuf, Vec<u8>)>) {
    std::fs::remove_dir_all(paths.data_dir.join("agent-test/history")).unwrap();
    for (path, bytes) in saved {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_reports_a_running_child_only_after_the_parent_stops() {
    let server = FakeServer::start(vec![
        (held(), Duration::ZERO),
        (assistant_events("The child was lost."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config, events, root) = setup("lost-clean-child", &server).await;
    let children = FakeServer::start(vec![(held(), Duration::ZERO)]).await;
    add_test_provider(&config, "provider-2", "Children", &children.base_url);
    let subagent = child(&service, root);
    let snapshot = service.snapshot(subagent).unwrap();
    service
        .set_session_model(crate::AgentSessionModelSelectionRequest {
            session_id: subagent,
            generation: snapshot.generation,
            revision: snapshot.revision,
            provider_id: "provider-2".into(),
            model: "fixture-model".into(),
        })
        .await
        .unwrap();
    let root_turn = tokio::spawn({
        let service = service.clone();
        async move { service.start_turn(root, "Keep working.".into()).await }
    });
    requested(&server, 1).await;
    let child_turn = tokio::spawn({
        let driver = service.driver.clone();
        async move { driver.run_turn(subagent, "Keep working.".into()).await }
    });
    requested(&children, 1).await;
    stop(&service);
    assert!(root_turn.await.unwrap().is_err());
    assert!(child_turn.await.unwrap().is_err());
    assert_eq!(
        server.requests().len(),
        1,
        "no shutdown report sampled by the parent"
    );
    lost_agent(&service, root, subagent);
    server.hold.notify_one();
    drop(service);

    let service = open(&paths, &config, &events);
    service.start_plugins().await;
    assert_eq!(server.requests().len(), 1);
    assert_eq!(events.woken(root), 0);
    lost_agent(&service, root, subagent);
    service.start_turn(root, "Continue.".into()).await.unwrap();
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
    let request = String::from_utf8_lossy(&server.requests()[1].body).into_owned();
    assert!(
        request.contains("stopped when the Agent restarted"),
        "{request}"
    );
    stop(&service);
    drop(service);
    let service = open(&paths, &config, &events);
    service.core.activate_session(root).unwrap();
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_reports_the_lost_subtree_once_even_after_wait_agent_consumes_it() {
    let server = FakeServer::start(vec![(held(), Duration::ZERO)]).await;
    let (service, paths, config, events, root) = setup("lost-crash-subtree", &server).await;
    let subagent = child(&service, root);
    let grandchild = child(&service, subagent);
    let turn = tokio::spawn({
        let driver = service.driver.clone();
        async move { driver.run_turn(grandchild, "Keep working.".into()).await }
    });
    requested(&server, 1).await;
    service.flush_for_diagnostics().unwrap();
    let saved = rollouts(&paths.data_dir.join("agent-test/history/active"));
    stop(&service);
    assert!(turn.await.unwrap().is_err());
    drop(service);
    restore(&paths, saved);

    let service = open(&paths, &config, &events);
    service.start_plugins().await;
    assert_eq!(
        server.requests().len(),
        1,
        "recovery starts no model request"
    );
    assert_eq!(events.woken(root), 0);
    lost_agent(&service, root, subagent);
    assert_eq!(service.snapshot(subagent).unwrap().pending_wake_events, 0);
    let taken = service
        .core
        .take_wake_events(root, AgentWakeFilter::SubagentStatuses)
        .await
        .unwrap();
    assert_eq!(taken.len(), 1);
    stop(&service);
    drop(service);
    let service = open(&paths, &config, &events);
    service.core.activate_session(root).unwrap();
    assert_eq!(
        service.snapshot(root).unwrap().pending_wake_events,
        0,
        "an already interrupted turn is not newly lost on another recovery"
    );
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn terminal_loss_survives_clean_stop_and_crash_without_waking_at_start() {
    for crash in [false, true] {
        let server = FakeServer::start(vec![(
            assistant_events("The terminal was lost."),
            Duration::ZERO,
        )])
        .await;
        let (service, paths, config, events, root) = setup("lost-terminal", &server).await;
        let sink = service
            .core
            .execution_progress_sink(root, Some("model-exec".into()));
        sink.report(AgentToolProgressUpdate::ExecutionStarted {
            execution_id: 77,
            generation: 1,
            action: hanafubuki_agent_contracts::AgentActivityAction::fixed("exec_command"),
            status: AgentExecutionProgressStatus::Running,
        });
        service
            .core
            .flush_session_execution_progress(root)
            .await
            .unwrap();
        service.flush_for_diagnostics().unwrap();
        let saved = crash.then(|| rollouts(&paths.data_dir.join("agent-test/history/active")));
        stop(&service);
        drop(service);
        if let Some(saved) = saved {
            restore(&paths, saved);
        }
        let service = open(&paths, &config, &events);
        service.start_plugins().await;
        let queued = service.core.test_wake_queue(root).unwrap();
        assert_eq!(queued.len(), 1, "{queued:?}");
        assert!(matches!(
            queued[0].source,
            AgentWakeSource::Terminal {
                execution_id: 77,
                event: AgentTerminalWake::Lost,
                ..
            }
        ));
        assert_eq!(events.woken(root), 0);
        assert_eq!(server.requests().len(), 0);
        service.start_turn(root, "Continue.".into()).await.unwrap();
        assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
        stop(&service);
        drop(service);
        let service = open(&paths, &config, &events);
        service.core.activate_session(root).unwrap();
        assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
        stop(&service);
        let _ = std::fs::remove_dir_all(paths.app_data_dir);
    }
}
