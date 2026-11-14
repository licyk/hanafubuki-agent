use super::*;
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_kernel::{
    AgentActivityAction, AgentError, AgentExecutionProgressStatus, AgentToolProgressUpdate,
};
use hanafubuki_agent_process::executor::ExecutorManager;

#[tokio::test]
async fn background_terminal_use_cases_are_session_scoped() {
    use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;

    let (mut service, paths) =
        configured_service("background-terminal-scope", "http://127.0.0.1:1/v1");
    let process = FakeAgentExecutionProcessPort::shared();
    service.test_replace_executions(AgentExecutionService::with_process_port(process));
    let first = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let second = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    install_fake_background_execution(&service, first.id, "first-command");
    install_fake_background_execution(&service, second.id, "second-command");

    let started_event = service
        .events_since(first.id, None)
        .unwrap()
        .events
        .into_iter()
        .find(|event| matches!(event.kind, AgentEventKind::ExecutionProgress { .. }))
        .unwrap();
    assert_eq!(started_event.turn_id, None);
    assert_eq!(started_event.sequence.value(), 1);

    assert_eq!(
        service.list_background_terminals(first.id).unwrap().len(),
        1
    );
    assert_eq!(
        service.list_background_terminals(second.id).unwrap().len(),
        1
    );
    let stopped = service
        .stop_background_terminals_confirmed(first.id)
        .unwrap();
    assert_eq!(stopped.disposed.len(), 1);
    assert!(stopped.retained.is_empty());
    assert!(service
        .list_background_terminals(first.id)
        .unwrap()
        .is_empty());
    assert_eq!(
        service.list_background_terminals(second.id).unwrap().len(),
        1
    );
    assert_eq!(
        service.snapshot(first.id).unwrap().recent_activities[0].phase,
        hanafubuki_agent_kernel::AgentActivityPhase::Cancelled
    );
    let terminal_events = service.events_since(first.id, None).unwrap().events;
    assert!(terminal_events.iter().all(|event| {
        !matches!(event.kind, AgentEventKind::ExecutionProgress { .. }) || event.turn_id.is_none()
    }));

    assert!(service
        .stop_background_terminals_confirmed(second.id)
        .unwrap()
        .retained
        .is_empty());
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn cleanup_failure_blocks_archive_and_retries_the_same_execution() {
    use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;

    let (mut service, paths) =
        configured_service("background-terminal-archive", "http://127.0.0.1:1/v1");
    let process = FakeAgentExecutionProcessPort::shared();
    service.test_replace_executions(AgentExecutionService::with_process_port(process.clone()));
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let execution = install_fake_background_execution(&service, session.id, "retained-command");
    let before = service.snapshot(session.id).unwrap();
    process.fail_next_terminations(1);

    assert_eq!(
        service.archive_session(session.id, before.generation, before.revision),
        Err(AgentError::SessionMutationNotConfirmed)
    );
    let retained = service.list_background_terminals(session.id).unwrap();
    assert_eq!(retained[0].execution_id, execution.execution_id);
    assert_eq!(retained[0].generation, execution.generation);
    assert!(retained[0].cleanup_required);

    let retry = service.snapshot(session.id).unwrap();
    let archived = service
        .archive_session(session.id, retry.generation, retry.revision)
        .unwrap();
    assert_eq!(archived.lifecycle, AgentSessionLifecycle::Archived);
    assert_eq!(process.session_count(), 0);
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn cleanup_failure_cancels_prepared_delete_and_retry_confirms_removal() {
    use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;

    let (mut service, paths) =
        configured_service("background-terminal-delete", "http://127.0.0.1:1/v1");
    let process = FakeAgentExecutionProcessPort::shared();
    service.test_replace_executions(AgentExecutionService::with_process_port(process.clone()));
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let execution = install_fake_background_execution(&service, session.id, "delete-command");
    let before = service.snapshot(session.id).unwrap();
    process.fail_next_terminations(1);

    assert_eq!(
        service.delete_session_confirmed(session.id, before.generation, before.revision),
        Err(AgentError::SessionMutationNotConfirmed)
    );
    let retained = service.list_background_terminals(session.id).unwrap();
    assert_eq!(retained[0].execution_id, execution.execution_id);
    let retry = service.snapshot(session.id).unwrap();
    let deleted = service
        .delete_session_confirmed(session.id, retry.generation, retry.revision)
        .unwrap();
    assert_eq!(deleted.session_id, session.id);
    assert_eq!(process.session_count(), 0);
    assert_eq!(
        service.snapshot(session.id),
        Err(AgentError::SessionNotFound)
    );
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn shutdown_reports_retained_background_process_authority() {
    use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;

    let (mut service, paths) =
        configured_service("background-terminal-shutdown", "http://127.0.0.1:1/v1");
    let process = FakeAgentExecutionProcessPort::shared();
    service.test_replace_executions(AgentExecutionService::with_process_port(process.clone()));
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    install_fake_background_execution(&service, session.id, "retained-on-shutdown");
    process.fail_next_terminations(1);

    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.worker.contains("agent-execution")));
    assert_eq!(process.session_count(), 1);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn startup_terminalizes_execution_activity_without_rebuilding_process_authority() {
    let (service, paths, config) =
        service_with_config("background-terminal-recovery", |_, config| {
            configure_test_provider(config, "http://127.0.0.1:1/v1", None);
        });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .core
        .report_session_execution_progress(
            session.id,
            AgentToolProgressUpdate::ExecutionStarted {
                execution_id: 77,
                generation: 4,
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            },
        )
        .unwrap();
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .active_activities
            .len(),
        1
    );
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    drop(service);

    let restarted = AgentApplicationService::new(
        &paths,
        config,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap();
    let recovered = restarted.snapshot(session.id).unwrap();
    assert!(recovered.active_activities.is_empty());
    let recovered_execution = recovered
        .recent_activities
        .iter()
        .find(|activity| activity.execution.is_some())
        .unwrap();
    assert_eq!(
        recovered_execution.phase,
        hanafubuki_agent_kernel::AgentActivityPhase::Interrupted
    );
    assert!(restarted
        .list_background_terminals(session.id)
        .unwrap()
        .is_empty());
    assert_eq!(
        restarted.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
