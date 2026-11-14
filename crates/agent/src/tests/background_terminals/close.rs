use super::*;

async fn running(
    name: &str,
    count: usize,
    side: bool,
) -> (
    AgentApplicationService,
    TestPaths,
    AgentSessionId,
    RecordedBackgroundEvents,
    FakeServer,
    std::path::PathBuf,
) {
    let go = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_file(&go);
    let args = exec_arguments(&waits_for(&go), 500);
    let calls = (0..count)
        .map(|index| (format!("call-{index}"), "exec_command", args.clone()))
        .collect::<Vec<_>>();
    let calls = calls
        .iter()
        .map(|(id, name, args)| (id.as_str(), *name, args.as_str()))
        .collect::<Vec<_>>();
    let mut replies = vec![
        (tool_calls_events(&calls), Duration::ZERO),
        (
            assistant_events("The commands are running."),
            Duration::ZERO,
        ),
        (
            assistant_events("The commands were closed."),
            Duration::ZERO,
        ),
    ];
    if side {
        replies.insert(0, (assistant_events("Ready."), Duration::ZERO));
    }
    let server = FakeServer::start(replies).await;
    let (service, paths, root, events) = waking(name, &server).await;
    let root = if side {
        let initial = person(&service, root, "Start the conversation.").await;
        ended(&service, initial).await;
        let side = service.open_side_conversation(root).unwrap().id;
        service
            .set_session_permission_mode(
                side,
                hanafubuki_agent_approval::AgentPermissionMode::FullAccess,
            )
            .unwrap();
        service
            .set_session_sandbox_mode(side, AgentSandboxMode::FullAccess)
            .unwrap();
        side
    } else {
        root
    };
    let turn = person(&service, root, "Run the commands.").await;
    ended(&service, turn).await;
    (service, paths, root, events, server, go)
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_three_terminals_delivers_three_updates_in_one_woken_turn() {
    let (service, paths, root, events, server, _) =
        running("close-three-terminals", 3, false).await;
    let result = service.stop_background_terminals_confirmed(root).unwrap();
    assert_eq!(result.disposed.len(), 3);
    assert!(result.retained.is_empty());
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 3);
    settled(&service, &events).await;
    assert!(events.woken(root) >= 1);
    let turn = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.events.len(), 3);
    assert!(turn.events.iter().all(|event| matches!(
        event.source,
        crate::AgentWakeSource::Terminal {
            event: crate::AgentTerminalWake::StoppedByUser { .. },
            ..
        }
    )));
    ended(&service, turn.turn).await;
    let request = body(&server, 2);
    assert!(request.contains("stopped by the user"));
    assert!(service.list_background_terminals(root).unwrap().is_empty());
    assert!(service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .is_none());
    assert_eq!(server.requests().len(), 3);
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_an_exited_terminal_replaces_its_exit_with_the_persons_close() {
    let (service, paths, root, events, server, go) =
        running("close-exited-terminal", 1, false).await;
    let terminal = service.list_background_terminals(root).unwrap()[0].execution_id;
    let other = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    assert!(matches!(
        service.close_background_terminal(other, terminal),
        Err(AgentError::InvalidInput(_))
    ));
    assert_eq!(service.list_background_terminals(root).unwrap().len(), 1);
    std::fs::write(&go, "go").unwrap();
    wait_until("the exit is queued", || {
        service.snapshot(root).unwrap().pending_wake_events == 1
    })
    .await;
    service.close_background_terminal(root, terminal).unwrap();
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 1);
    let turn = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    ended(&service, turn.turn).await;
    let request = body(&server, 2);
    assert!(request.contains("stopped by the user"), "{request}");
    assert!(!request.contains("exited with code 3 after"), "{request}");
    settled(&service, &events).await;
    assert!(service.list_background_terminals(root).unwrap().is_empty());
    finish(service, paths);
    let _ = std::fs::remove_file(go);
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_and_delete_stop_real_terminals_without_waking_the_owner() {
    for delete in [false, true] {
        let name = if delete {
            "quiet-delete-terminal"
        } else {
            "quiet-archive-terminal"
        };
        let (service, paths, root, events, _, _) = running(name, 1, false).await;
        let before = service.snapshot(root).unwrap();
        if delete {
            service
                .delete_session_confirmed(root, before.generation, before.revision)
                .unwrap();
        } else {
            service
                .archive_session(root, before.generation, before.revision)
                .unwrap();
            assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
        }
        settled(&service, &events).await;
        assert_eq!(events.woken(root), 0);
        finish(service, paths);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn side_close_stops_its_real_terminal_without_waking_any_owner() {
    let (service, paths, side, events, _, _) = running("quiet-side-terminal", 1, true).await;
    let root = service.snapshot(side).unwrap().parent_session_id.unwrap();
    assert_eq!(service.list_background_terminals(side).unwrap().len(), 1);
    service.close_side_conversation(side).await.unwrap();
    assert_eq!(service.snapshot(side), Err(AgentError::SessionNotFound));
    settled(&service, &events).await;
    assert_eq!(events.woken(side), 0);
    assert_eq!(events.woken(root), 0);
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_model_and_foreground_bang_commands_reports_only_the_model_terminal() {
    let (service, paths, root, _, _server, go) = running("close-command-origins", 1, false).await;
    let model_terminal = service.list_background_terminals(root).unwrap()[0]
        .execution_id
        .value();
    let shell = service
        .submit_user_shell_command_with_sink(root, waits_for(&go), Arc::new(Discard))
        .await
        .unwrap();
    wait_until("the foreground command starts", || {
        service.list_background_terminals(root).unwrap().len() == 2
    })
    .await;
    let stopped = service.stop_background_terminals_confirmed(root).unwrap();
    assert_eq!(stopped.disposed.len(), 2);
    ended(&service, shell).await;
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 1);
    let resumed = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed.events.len(), 1);
    assert!(
        matches!(resumed.events[0].source, crate::AgentWakeSource::Terminal { execution_id, event: crate::AgentTerminalWake::StoppedByUser { .. }, .. } if execution_id == model_terminal)
    );
    ended(&service, resumed.turn).await;
    finish(service, paths);
}
