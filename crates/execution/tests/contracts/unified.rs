use super::*;

#[test]
#[cfg(unix)]
fn unified_exec_completes_reads_and_disposes_in_one_call() {
    let process = FakeAgentExecutionProcessPort::shared();
    process.finish_on_start("unified", 0);
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(21).unwrap();
    let request = exec_request(false);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request.clone(),
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let result = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            request.yield_time_ms,
            request.max_output_bytes,
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(result.output, "unified");
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.session_id, None);
    assert_eq!(process.session_count(), 0);
}

#[test]
#[cfg(unix)]
fn unified_exec_yields_then_empty_write_stdin_polls_to_terminal_disposal() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(22).unwrap();
    let mut request = exec_request(true);
    request.yield_time_ms = 1;
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request.clone(),
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let yielded = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            request.yield_time_ms,
            request.max_output_bytes,
            &CancellationToken::default(),
        )
        .unwrap();
    let session_id = yielded.session_id.unwrap();
    assert_eq!(yielded.exit_code, None);
    assert_eq!(process.starts()[0].io_mode, ExecutorIoMode::Pty);

    process.finish("done", 7);
    let completed = service
        .write_stdin(
            &scope,
            WriteStdinRequest {
                session_id: hanafubuki_agent_execution::AgentExecutionId::from_untrusted(
                    session_id,
                )
                .unwrap(),
                chars: String::new(),
                yield_time_ms: 10,
                max_output_bytes: Some(4096),
            },
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(completed.output, "done");
    assert_eq!(completed.exit_code, Some(7));
    assert_eq!(completed.session_id, None);
    assert_eq!(process.session_count(), 0);
    assert!(process.writes().is_empty());
    assert_eq!(process.interrupt_count(), 0);
}

#[test]
#[cfg(unix)]
fn pipe_ctrl_c_interrupts_without_writing_stdin() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(32).unwrap();
    let request = exec_request(false);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();

    let foreign = AgentExecutionScope::for_agent_session(35).unwrap();
    assert!(matches!(
        service.write_stdin(
            &foreign,
            WriteStdinRequest {
                session_id: started.execution_id,
                chars: "\u{3}".to_string(),
                yield_time_ms: 1,
                max_output_bytes: Some(4096),
            },
            &CancellationToken::default(),
        ),
        Err(AgentExecutionError::CrossScope)
    ));
    assert_eq!(process.interrupt_count(), 0);

    let result = service
        .write_stdin(
            &scope,
            WriteStdinRequest {
                session_id: started.execution_id,
                chars: "\u{3}".to_string(),
                yield_time_ms: 1,
                max_output_bytes: Some(4096),
            },
            &CancellationToken::default(),
        )
        .unwrap();

    assert_eq!(result.session_id, Some(started.execution_id.value()));
    assert_eq!(process.interrupt_count(), 1);
    assert!(process.writes().is_empty());
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
#[cfg(unix)]
fn pipe_rejects_ordinary_input_before_process_write() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(33).unwrap();
    let request = exec_request(false);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();

    assert!(matches!(
        service.write_stdin(
            &scope,
            WriteStdinRequest {
                session_id: started.execution_id,
                chars: "input".to_string(),
                yield_time_ms: 1,
                max_output_bytes: Some(4096),
            },
            &CancellationToken::default(),
        ),
        Err(AgentExecutionError::NotInteractive)
    ));
    assert_eq!(process.interrupt_count(), 0);
    assert!(process.writes().is_empty());
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
#[cfg(unix)]
fn pty_ctrl_c_is_written_as_raw_input_instead_of_pipe_interrupt() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(34).unwrap();
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

    service
        .write_stdin(
            &scope,
            WriteStdinRequest {
                session_id: started.execution_id,
                chars: "\u{3}".to_string(),
                yield_time_ms: 1,
                max_output_bytes: Some(4096),
            },
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(process.writes(), [vec![3]]);
    assert_eq!(process.interrupt_count(), 0);
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
#[cfg(unix)]
fn resize_is_bound_to_the_exact_tty_session_generation_and_scope() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(36).unwrap();
    let foreign = AgentExecutionScope::for_agent_session(37).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let tty = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(tty.interactive);
    let resized = service
        .resize_interactive(&scope, tty.execution_id, tty.generation, 120, 40)
        .unwrap();
    assert_eq!((resized.cols, resized.rows), (120, 40));
    assert!(matches!(
        service.resize_interactive(
            &scope,
            tty.execution_id,
            AgentExecutionGeneration::from_untrusted(99).unwrap(),
            120,
            40
        ),
        Err(AgentExecutionError::StaleGeneration)
    ));
    assert!(matches!(
        service.resize_interactive(&foreign, tty.execution_id, tty.generation, 120, 40),
        Err(AgentExecutionError::CrossScope)
    ));

    let request = exec_request(false);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let pipe = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(!pipe.interactive);
    assert!(!process.starts()[1].stream_stdin);
    assert!(matches!(
        service.resize_interactive(&scope, pipe.execution_id, pipe.generation, 120, 40),
        Err(AgentExecutionError::NotInteractive)
    ));
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
#[cfg(unix)]
fn terminal_unified_exec_retains_cleanup_obligation_when_disposal_fails() {
    let process = FakeAgentExecutionProcessPort::shared();
    process.finish_on_start("done", 0);
    process.fail_next_disposals(1);
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(23).unwrap();
    let request = exec_request(false);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request.clone(),
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(matches!(
        service.wait_unified_execution(
            &scope,
            started.execution_id,
            request.yield_time_ms,
            request.max_output_bytes,
            &CancellationToken::default(),
        ),
        Err(AgentExecutionError::CleanupRetained { .. })
    ));
    assert_eq!(service.registry_count(), 1);
    assert_eq!(process.session_count(), 1);
}

#[test]
#[cfg(unix)]
fn unified_exec_wait_cancellation_preserves_the_committed_background_terminal() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(24).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request.clone(),
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(started.kind, AgentExecutionKind::UnifiedExec);
    let cancelled = CancellationToken::default();
    cancelled.cancel();
    assert!(matches!(
        service.wait_unified_execution(
            &scope,
            started.execution_id,
            request.yield_time_ms,
            request.max_output_bytes,
            &cancelled,
        ),
        Err(AgentExecutionError::Cancelled)
    ));
    assert_eq!(process.session_count(), 1);
    assert_eq!(service.list_background_terminals(&scope).unwrap().len(), 1);
    let stopped = service.stop_background_terminals_confirmed(&scope);
    assert_eq!(stopped.disposed.len(), 1);
    assert!(stopped.retained.is_empty());
}

#[test]
#[cfg(unix)]
fn empty_write_stdin_cancellation_preserves_the_committed_background_terminal() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(25).unwrap();
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
    let cancelled = CancellationToken::default();
    cancelled.cancel();
    assert!(matches!(
        service.write_stdin(
            &scope,
            WriteStdinRequest {
                session_id: started.execution_id,
                chars: String::new(),
                yield_time_ms: 10,
                max_output_bytes: Some(4096),
            },
            &cancelled,
        ),
        Err(AgentExecutionError::Cancelled)
    ));
    assert_eq!(process.session_count(), 1);
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
#[cfg(unix)]
fn nonempty_write_stdin_wait_cancellation_preserves_the_committed_background_terminal() {
    let process = FakeAgentExecutionProcessPort::shared();
    process.set_write_delay_ms(40);
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(30).unwrap();
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
    let cancellation = CancellationToken::default();
    let waiter_service = service.clone();
    let waiter_scope = scope.clone();
    let waiter_cancellation = cancellation.clone();
    let waiter = std::thread::spawn(move || {
        waiter_service.write_stdin(
            &waiter_scope,
            WriteStdinRequest {
                session_id: started.execution_id,
                chars: "input".to_string(),
                yield_time_ms: 5_000,
                max_output_bytes: Some(4096),
            },
            &waiter_cancellation,
        )
    });
    std::thread::sleep(Duration::from_millis(10));
    cancellation.cancel();
    assert!(matches!(
        waiter.join().unwrap(),
        Err(AgentExecutionError::Cancelled)
    ));
    assert_eq!(process.writes(), [b"input".to_vec()]);
    assert_eq!(service.list_background_terminals(&scope).unwrap().len(), 1);
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
fn output_is_bounded_wait_is_cancellable_and_scope_is_exact() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let foreign = AgentExecutionScope::for_agent_session(2).unwrap();
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
    process.finish(vec![b'x'; 64], 0);

    let output = service
        .read_output(&scope, started.execution_id, Some(0), Some(16))
        .unwrap();
    assert_eq!(output.chunks[0].text.len(), 16);
    assert!(output.truncated);
    assert!(matches!(
        service.read_output(&foreign, started.execution_id, None, Some(16)),
        Err(AgentExecutionError::CrossScope)
    ));
    assert!(matches!(
        service.wait(
            &foreign,
            started.execution_id,
            10,
            &CancellationToken::default()
        ),
        Err(AgentExecutionError::CrossScope)
    ));
    assert!(matches!(
        service.terminate(&foreign, started.execution_id),
        Err(AgentExecutionError::CrossScope)
    ));
    assert!(matches!(
        service.dispose(&foreign, started.execution_id),
        Err(AgentExecutionError::CrossScope)
    ));
    let cancellation = CancellationToken::default();
    cancellation.cancel();
    assert!(matches!(
        service.wait(&scope, started.execution_id, 100, &cancellation),
        Err(AgentExecutionError::Cancelled)
    ));
}

#[test]
fn thirty_second_wait_is_released_immediately_by_cancellation_notification() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process);
    let scope = AgentExecutionScope::for_agent_session(31).unwrap();
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
    let cancellation = CancellationToken::default();
    let waiter_service = service.clone();
    let waiter_scope = scope.clone();
    let waiter_cancellation = cancellation.clone();
    let began = std::time::Instant::now();
    let waiter = std::thread::spawn(move || {
        waiter_service.wait(
            &waiter_scope,
            started.execution_id,
            30_000,
            &waiter_cancellation,
        )
    });

    std::thread::sleep(Duration::from_millis(25));
    cancellation.cancel();
    assert!(matches!(
        waiter.join().unwrap(),
        Err(AgentExecutionError::Cancelled)
    ));
    assert!(began.elapsed() < Duration::from_secs(1));
    service.dispose(&scope, started.execution_id).unwrap();
}

/// The shell reads a command's output out of this result, so it has to be able
/// to tell one apart from every other structured tool payload it may receive.
/// Without the tag it could only guess from field names, which is how the
/// transcript ended up printing the whole result envelope as JSON.
#[test]
fn a_unified_exec_result_is_self_describing_on_the_wire() {
    let value = serde_json::to_value(UnifiedExecResult {
        output: "hello\n".to_string(),
        exit_code: Some(0),
        session_id: None,
        wall_time_ms: 12,
        truncated: false,
    })
    .unwrap();

    assert_eq!(value["type"], "unified_exec");
    assert_eq!(value["output"], "hello\n");
    assert_eq!(value["exitCode"], 0);
    assert_eq!(value["sessionId"], serde_json::Value::Null);
    assert_eq!(value["truncated"], false);

    // The tag round-trips, so a retained result still deserializes.
    let restored: UnifiedExecResult = serde_json::from_value(value).unwrap();
    assert_eq!(restored.output, "hello\n");
    assert_eq!(restored.exit_code, Some(0));
}

#[cfg(unix)]
fn start_unified(
    service: &AgentExecutionService,
    scope: &AgentExecutionScope,
    request: &ExecCommandRequest,
) -> hanafubuki_agent_execution::AgentExecutionSnapshot {
    let action = service.prepare_exec_command_action(request).unwrap();
    service
        .start_exec_command(
            scope.clone(),
            request.clone(),
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap()
}

#[test]
#[cfg(unix)]
fn unified_exec_environment_is_non_interactive_unless_the_request_overrides_it() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(41).unwrap();
    let mut request = exec_request(false);
    request
        .environment
        .insert("PAGER".to_string(), "less".to_string());
    start_unified(&service, &scope, &request);

    let env = &process.starts()[0].env;
    for (key, value) in [
        ("NO_COLOR", "1"),
        ("TERM", "dumb"),
        ("GIT_PAGER", "cat"),
        ("LC_ALL", "C.UTF-8"),
        ("PAGER", "less"),
    ] {
        assert_eq!(env[key].as_deref(), Some(value), "{key}");
    }
}

#[test]
#[cfg(unix)]
fn unified_output_is_plain_text_and_an_unfinished_sequence_waits_for_the_next_poll() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(42).unwrap();
    let mut request = exec_request(true);
    request.yield_time_ms = 1;
    let started = start_unified(&service, &scope, &request);
    let process_id = format!("agent-command-{}", started.execution_id.value());

    let first = b"\x1b[31mred\x1b[0m\r\n50%\r60%\n\x1b[3".to_vec();
    process.set_output(&process_id, first.clone());
    let yielded = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            1,
            None,
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(yielded.output, "red\n60%\n");
    assert!(yielded.session_id.is_some());

    let mut full = first;
    full.extend_from_slice(b"2mgreen");
    process.finish(full, 0);
    let completed = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            100,
            None,
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(completed.output, "green");
    assert_eq!(completed.exit_code, Some(0));
}

#[test]
#[cfg(unix)]
fn finished_execution_is_disposed_only_after_its_observer_releases_it() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(43).unwrap();
    let request = exec_request(false);
    let started = start_unified(&service, &scope, &request);
    service
        .attach_observer(&scope, started.execution_id, started.generation)
        .unwrap();
    process.finish("done", 0);

    let observer = service.clone();
    let observer_scope = scope.clone();
    let release_after = Duration::from_millis(200);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(release_after);
        // Still readable: disposal has not happened yet.
        assert!(observer
            .read_output(&observer_scope, started.execution_id, Some(0), Some(64))
            .is_ok());
        observer.release_observer(&observer_scope, started.execution_id, started.generation);
    });
    let waited = std::time::Instant::now();
    let result = service
        .wait_unified_execution(
            &scope,
            started.execution_id,
            100,
            None,
            &CancellationToken::default(),
        )
        .unwrap();
    releaser.join().unwrap();
    assert!(waited.elapsed() >= release_after);
    assert_eq!(result.output, "done");
    assert_eq!(process.session_count(), 0);
}

/// A command without a `workdir` runs in the service's working directory,
/// never in the process's own, which `-C` does not change; a relative
/// `workdir` resolves against it. Without a working directory only an
/// absolute `workdir` runs.
#[test]
#[cfg(unix)]
fn a_command_runs_in_the_working_directory_not_the_processs() {
    let process = FakeAgentExecutionProcessPort::shared();
    process.finish_on_start("", 0);
    let service = AgentExecutionService::with_process_port(process.clone());
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("sub")).unwrap();
    let root = hanafubuki_agent_foundation::paths::ResolvedPath::resolve(workspace.path())
        .unwrap()
        .native()
        .to_path_buf();
    assert_ne!(Some(&root), std::env::current_dir().ok().as_ref());
    let mut request = exec_request(false);
    request.workdir = None;
    assert!(matches!(
        service.prepare_exec_command_action(&request),
        Err(AgentExecutionError::Resolution(_))
    ));

    service.set_working_directory(workspace.path().to_path_buf());
    for (workdir, expected) in [
        (None, root.clone()),
        (Some("sub"), root.join("sub")),
        (Some("sub/.."), root.clone()),
    ] {
        request.workdir = workdir.map(std::path::PathBuf::from);
        let action = service.prepare_exec_command_action(&request).unwrap();
        service
            .start_exec_command(
                AgentExecutionScope::for_agent_session(40).unwrap(),
                request.clone(),
                Some(&approve_action(&action)),
                &CancellationToken::default(),
            )
            .unwrap();
        assert_eq!(process.starts().last().unwrap().cwd, Some(expected));
    }
}
