use super::*;

#[test]
#[cfg(unix)]
fn confirmed_stop_releases_a_concurrent_model_wait_through_real_terminal_state() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(32).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let waiter_service = service.clone();
    let waiter_scope = scope.clone();
    let waiter = std::thread::spawn(move || {
        waiter_service.wait_unified_execution(
            &waiter_scope,
            started.execution_id,
            5_000,
            Some(4096),
            &CancellationToken::default(),
        )
    });
    std::thread::sleep(Duration::from_millis(10));
    let stopped = service.stop_background_terminals_confirmed(&scope);
    assert_eq!(stopped.disposed.len(), 1);
    assert!(matches!(
        waiter.join().unwrap(),
        Err(AgentExecutionError::NotFound)
    ));
    let empty = service.stop_background_terminals_confirmed(&scope);
    assert!(empty.disposed.is_empty());
    assert!(empty.retained.is_empty());
}

#[test]
#[cfg(unix)]
fn background_terminal_listing_is_scope_and_kind_bound_with_bounded_recent_output() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(26).unwrap();
    let foreign = AgentExecutionScope::for_agent_session(27).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    process.set_output("agent-command-1", b"one\ntwo\nthree\nfour\n".to_vec());
    let ordinary = command_request();
    service
        .start(
            scope.clone(),
            ordinary.clone(),
            Some(&approved(&ordinary)),
            &CancellationToken::default(),
        )
        .unwrap();
    let terminals = service.list_background_terminals(&scope).unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].execution_id, started.execution_id);
    assert_eq!(terminals[0].command, "printf unified");
    assert_eq!(terminals[0].recent_output, ["two", "three", "four"]);
    let encoded = serde_json::to_value(&terminals[0]).unwrap();
    assert!(encoded.get("executionId").is_some());
    assert!(encoded.get("createdAtMs").is_some());
    assert!(encoded.get("cleanupRequired").is_some());
    for forbidden in ["receipt", "owner", "executorId", "environment"] {
        assert!(!encoded.to_string().contains(forbidden));
    }
    assert!(service
        .list_background_terminals(&foreign)
        .unwrap()
        .is_empty());
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
#[cfg(unix)]
fn background_stop_retains_failed_cleanup_and_retries_the_same_identity() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(28).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    process.fail_next_terminations(1);
    let failed = service.stop_background_terminals_confirmed(&scope);
    assert!(failed.disposed.is_empty());
    assert_eq!(failed.retained.len(), 1);
    assert_eq!(failed.retained[0].execution_id, started.execution_id);
    assert_eq!(service.registry_count(), 1);
    let retried = service.stop_background_terminals_confirmed(&scope);
    assert_eq!(retried.disposed.len(), 1);
    assert!(retried.retained.is_empty());
    assert_eq!(service.registry_count(), 0);
}
