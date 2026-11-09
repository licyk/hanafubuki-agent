use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::*;
use crate::model::{AgentSessionConfig, AgentSessionLifecycle};
use crate::service::persistence::publish_persisted_revision;
use crate::service::AgentService;

mod fixtures {
    use super::*;

    pub(super) fn service_with_sessions(count: usize) -> (AgentService, Vec<AgentSessionId>) {
        let service = AgentService::without_default_model_with_history_store(
            crate::AgentKernelRegistries::default(),
            Arc::new(crate::EphemeralAgentHistoryStore::default()),
        );
        let sessions = (0..count)
            .map(|_| {
                service
                    .create_model_session(
                        config(),
                        hanafubuki_agent_model_api::ModelApiProtocol::Responses,
                        "http://127.0.0.1:1/v1",
                        None,
                        Vec::new(),
                        hanafubuki_agent_model_api::ModelRetryPolicy::default(),
                        hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
                        crate::AgentSessionWorkspace::default(),
                    )
                    .unwrap()
                    .id
            })
            .collect();
        (service, sessions)
    }

    fn config() -> AgentSessionConfig {
        AgentSessionConfig {
            provider_id: "provider-1".to_string(),
            provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
            model: "fixture".to_string(),
            instructions: "fixture".to_string(),
            mode: crate::AgentSessionMode::Default,
            reasoning_effort: None,
            context_window_tokens: None,
            input_capabilities: Default::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability:
                hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
            sampling_retry_policy: crate::AgentSamplingRetryPolicy::default(),
            compaction_strategy: Default::default(),
        }
    }

    /// Marks the session's current revision as staged, as a commit in
    /// flight does, and returns that revision.
    pub(super) fn stage(cell: &SessionCell) -> u64 {
        let mut record = cell.lock().unwrap();
        let revision = record.status.revision;
        record.status.persisting_revision = Some(revision);
        revision
    }
}

use fixtures::{service_with_sessions, stage};

#[test]
fn a_session_held_by_long_work_does_not_hold_back_another() {
    let (service, sessions) = service_with_sessions(2);
    let (busy, other) = (sessions[0], sessions[1]);
    let busy_cell = service.inner.sessions.get(busy).unwrap();
    let held = busy_cell.lock().unwrap();

    let (sender, receiver) = mpsc::channel();
    let worker = {
        let service = service.clone();
        thread::spawn(move || {
            let snapshot = service.snapshot(other).map(|snapshot| snapshot.revision);
            let mutated = service
                .set_session_turn_budget_policy(other, crate::AgentTurnBudgetPolicy::default());
            let _ = sender.send((snapshot, mutated));
        })
    };
    let (snapshot, mutated) = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("another session's snapshot waited for the held session");
    assert!(snapshot.is_ok());
    assert_eq!(mutated, Ok(()));
    worker.join().unwrap();

    let (sender, receiver) = mpsc::channel();
    let waiter = {
        let service = service.clone();
        thread::spawn(move || {
            let _ = sender.send(service.snapshot(busy).map(|snapshot| snapshot.revision));
        })
    };
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    drop(held);
    assert!(receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .is_ok());
    waiter.join().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn an_async_wait_for_persistence_resumes_when_the_commit_settles() {
    let (service, sessions) = service_with_sessions(1);
    let cell = service.inner.sessions.get(sessions[0]).unwrap();
    let revision = stage(&cell);

    let waiting = tokio::spawn({
        let cell = cell.clone();
        async move {
            cell.lock_after_persistence()
                .await
                .map(|record| record.status.revision)
        }
    });
    let blocking = thread::spawn({
        let cell = cell.clone();
        move || {
            cell.lock_after_persistence_blocking()
                .map(|record| record.status.revision)
        }
    });
    // The single runtime thread keeps running while the task waits.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(!waiting.is_finished());
    assert!(!blocking.is_finished());

    assert!(publish_persisted_revision(&cell, revision).is_none());

    assert_eq!(waiting.await.unwrap(), Ok(revision));
    assert_eq!(blocking.join().unwrap(), Ok(revision));
}

#[tokio::test(flavor = "current_thread")]
async fn an_async_wait_for_a_terminal_state_resumes_on_the_terminal_signal() {
    let (service, sessions) = service_with_sessions(1);
    let cell = service.inner.sessions.get(sessions[0]).unwrap();
    let archived = |record: &SessionRecord| {
        Ok((record.status.lifecycle == AgentSessionLifecycle::Archived).then_some(()))
    };

    let waiting = tokio::spawn({
        let cell = cell.clone();
        async move { cell.wait_until(archived).await }
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(!waiting.is_finished());

    cell.lock().unwrap().status.lifecycle = AgentSessionLifecycle::Archived;
    cell.notify_terminal();

    assert_eq!(waiting.await.unwrap(), Ok(()));
}

#[tokio::test(flavor = "current_thread")]
async fn removing_a_session_releases_its_waiters() {
    let (service, sessions) = service_with_sessions(1);
    let cell = service.inner.sessions.get(sessions[0]).unwrap();
    stage(&cell);

    let waiting = tokio::spawn({
        let cell = cell.clone();
        async move { cell.lock_after_persistence().await.map(|_| ()) }
    });
    let blocking = thread::spawn({
        let cell = cell.clone();
        move || cell.lock_after_persistence_blocking().map(|_| ())
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(!waiting.is_finished());

    service.inner.sessions.remove(cell.lock().unwrap());

    assert_eq!(waiting.await.unwrap(), Err(AgentError::SessionNotFound));
    assert_eq!(blocking.join().unwrap(), Err(AgentError::SessionNotFound));
    assert!(service.inner.sessions.find(sessions[0]).is_none());
}

#[test]
fn a_blocking_wait_ends_at_its_deadline() {
    let (service, sessions) = service_with_sessions(1);
    let cell = service.inner.sessions.get(sessions[0]).unwrap();
    let started = Instant::now();

    let waited = cell.wait_until_blocking(started + Duration::from_millis(50), |_| Ok(None::<()>));

    assert_eq!(waited, Ok(None));
    assert!(started.elapsed() >= Duration::from_millis(50));
}

#[test]
fn a_commit_deferred_behind_a_staged_one_is_staged_when_it_settles() {
    let (service, sessions) = service_with_sessions(1);
    let cell = service.inner.sessions.get(sessions[0]).unwrap();
    let revision = stage(&cell);
    let (ran, staged_on) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(None)));

    let deferred = cell.stage_or_defer(Box::new({
        let (ran, staged_on) = (ran.clone(), staged_on.clone());
        move |_, record| {
            ran.store(true, Ordering::SeqCst);
            // It runs under the lock that cleared the settled commit.
            *staged_on.lock() = Some(record.status.persisting_revision);
            None
        }
    }));

    assert!(deferred.is_none());
    assert!(
        !ran.load(Ordering::SeqCst),
        "staged while a commit was in flight"
    );
    assert!(publish_persisted_revision(&cell, revision).is_none());
    assert!(ran.load(Ordering::SeqCst));
    assert_eq!(*staged_on.lock(), Some(None));
}

#[test]
fn a_commit_with_nothing_staged_is_staged_at_once() {
    let (service, sessions) = service_with_sessions(1);
    let cell = service.inner.sessions.get(sessions[0]).unwrap();
    let ran = Arc::new(AtomicBool::new(false));

    let staged = cell.stage_or_defer(Box::new({
        let ran = ran.clone();
        move |_, _| {
            ran.store(true, Ordering::SeqCst);
            None
        }
    }));

    assert!(staged.is_none());
    assert!(ran.load(Ordering::SeqCst));
}

#[test]
fn removing_a_session_drops_its_deferred_commits() {
    let (service, sessions) = service_with_sessions(1);
    let cell = service.inner.sessions.get(sessions[0]).unwrap();
    let revision = stage(&cell);
    let ran = Arc::new(AtomicBool::new(false));
    let _ = cell.stage_or_defer(Box::new({
        let ran = ran.clone();
        move |_, _| {
            ran.store(true, Ordering::SeqCst);
            None
        }
    }));

    service.inner.sessions.remove(cell.lock().unwrap());

    assert!(publish_persisted_revision(&cell, revision).is_none());
    assert!(!ran.load(Ordering::SeqCst));
}
