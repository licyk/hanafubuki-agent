use super::*;

#[test]
fn cancellation_before_spawn_releases_every_reservation() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let request = command_request();
    let approval = approved(&request);
    let cancellation = CancellationToken::default();
    cancellation.cancel();

    assert!(matches!(
        service.start(
            AgentExecutionScope::for_agent_session(1).unwrap(),
            request,
            Some(&approval),
            &cancellation
        ),
        Err(AgentExecutionError::Cancelled)
    ));
    assert_eq!(service.registry_count(), 0);
    assert_eq!(process.reservation_count(), 0);
    assert_eq!(process.session_count(), 0);
}

#[test]
fn executor_timeout_is_reported_without_implicit_disposal() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let request = command_request();
    let approval = approved(&request);
    let started = service
        .start(
            scope.clone(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    process.time_out();

    let outcome = service
        .wait(
            &scope,
            started.execution_id,
            100,
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(outcome.completed);
    assert_eq!(outcome.snapshot.status, AgentExecutionStatus::TimedOut);
    assert_eq!(service.registry_count(), 1);
    service.dispose(&scope, started.execution_id).unwrap();
}

#[test]
fn termination_failure_retains_the_execution_for_exact_retry() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let request = command_request();
    let approval = approved(&request);
    let started = service
        .start(
            scope.clone(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    process.fail_next_terminations(1);

    assert!(matches!(
        service.terminate(&scope, started.execution_id),
        Err(AgentExecutionError::CleanupRetained { .. })
    ));
    assert_eq!(service.registry_count(), 1);
    let terminated = service.terminate(&scope, started.execution_id).unwrap();
    assert_eq!(terminated.status, AgentExecutionStatus::Cancelled);
    service.dispose(&scope, started.execution_id).unwrap();
}

#[test]
fn cleanup_failure_retains_exact_receipt_and_retry_disposes_it() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let request = command_request();
    let approval = approved(&request);
    let started = service
        .start(
            scope.clone(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    process.fail_next_disposals(1);

    assert!(matches!(
        service.dispose(&scope, started.execution_id),
        Err(AgentExecutionError::CleanupRetained { .. })
    ));
    assert_eq!(service.registry_count(), 1);
    assert_eq!(process.session_count(), 1);
    service.dispose(&scope, started.execution_id).unwrap();
    assert_eq!(service.registry_count(), 0);
    assert_eq!(process.session_count(), 0);
}
