use super::*;

#[test]
fn start_uses_clear_environment_and_never_returns_process_identity() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let request = command_request();
    let approval = approved(&request);
    let snapshot = service
        .start(
            scope,
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();

    assert_eq!(snapshot.execution_id.value(), 1);
    let starts = process.starts();
    assert_eq!(
        starts[0].environment_inheritance,
        EnvironmentInheritance::Clear
    );
    assert_eq!(starts[0].io_mode, ExecutorIoMode::Pipe);
    assert_eq!(
        starts[0].env["HANAFUBUKI_AGENT_TEST"].as_deref(),
        Some("present")
    );
    assert_eq!(starts[0].output_bytes_cap, Some(32));
    let encoded = serde_json::to_string(&snapshot).unwrap();
    assert!(!encoded.contains("executor"));
    assert!(!encoded.contains("receipt"));
}

#[test]
fn native_platform_smoke_runs_reads_and_confirmed_disposes_a_real_process() {
    let service =
        AgentExecutionService::new(hanafubuki_agent_process::executor::ExecutorManager::default());
    let scope = AgentExecutionScope::for_agent_session(77).unwrap();
    #[cfg(unix)]
    let argv = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "printf hanafubuki-native-smoke".to_string(),
    ];
    #[cfg(windows)]
    let argv = vec![
        "cmd.exe".to_string(),
        "/D".to_string(),
        "/S".to_string(),
        "/C".to_string(),
        "echo|set /p=hanafubuki-native-smoke".to_string(),
    ];
    let request = StartCommandRequest {
        argv,
        cwd: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        timeout_ms: Some(5_000),
        output_bytes_cap: Some(1024),
        origin: ApprovalOrigin::Model,
    };
    let approval = approved(&request);
    let started = service
        .start(
            scope.clone(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    let waited = service
        .wait(
            &scope,
            started.execution_id,
            5_000,
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(waited.completed);
    let output = service
        .read_output(&scope, started.execution_id, None, Some(1024))
        .unwrap();
    let bytes = output
        .chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>();
    assert!(bytes.contains("hanafubuki-native-smoke"));
    service.dispose(&scope, started.execution_id).unwrap();
}

#[test]
fn clean_environment_is_allowlisted_and_approval_is_bound_to_the_exact_plan() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let request = command_request();
    let approval = approved(&request);
    service
        .start(
            AgentExecutionScope::for_agent_session(1).unwrap(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    let clean = &process.starts()[0].env;
    assert!(clean.keys().any(|key| key.eq_ignore_ascii_case("PATH")));
    assert!(!clean.contains_key("CARGO_PKG_NAME"));

    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let original = command_request();
    let approval = approved(&original);
    let mut changed = original;
    changed.argv.push("different".to_string());
    assert!(matches!(
        service.start(
            scope,
            changed,
            Some(&approval),
            &CancellationToken::default()
        ),
        Err(AgentExecutionError::ApprovalMismatch)
    ));
    assert_eq!(process.session_count(), 0);
    assert_eq!(process.reservation_count(), 0);
}

#[test]
fn executable_lookup_is_canonical_before_reservation() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let mut request = command_request();
    request.argv[0] = if cfg!(windows) {
        "cmd.exe".to_string()
    } else {
        "echo".to_string()
    };
    let approval = approved(&request);
    service
        .start(
            AgentExecutionScope::for_agent_session(1).unwrap(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(std::path::Path::new(&process.starts()[0].command[0]).is_absolute());
}
