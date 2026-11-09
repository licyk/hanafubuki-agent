//! The session's permission mode and the approval of an action automatic
//! permission review denied.

use super::*;
use hanafubuki_agent_approval::AgentPermissionMode;
use hanafubuki_agent_process::executor::ExecutorManager;

struct Discard;

impl AgentEventSink for Discard {
    fn emit(&self, _event: AgentEvent) {}
}

// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_permission_mode_replaces_the_configured_one_for_that_session_only() {
    // Each turn and the title request it starts take one answer.
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 4]).await;
    let (service, paths) = configured_service("session-permission-mode", &server.base_url);
    let configured = service
        .settings_view()
        .await
        .unwrap()
        .settings
        .permission_mode;
    assert_ne!(configured, AgentPermissionMode::FullAccess);
    let chosen = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let other = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    assert_eq!(service.session_permission_mode(chosen.id), Ok(None));

    service
        .set_session_permission_mode(chosen.id, AgentPermissionMode::FullAccess)
        .unwrap();

    assert_eq!(
        service.session_permission_mode(chosen.id),
        Ok(Some(AgentPermissionMode::FullAccess))
    );
    assert_eq!(service.session_permission_mode(other.id), Ok(None));
    for (session, expected) in [
        (chosen.id, AgentPermissionMode::FullAccess),
        (other.id, configured),
    ] {
        service
            .submit_turn_with_sink(
                session,
                AgentTurnRequest {
                    text: "Hello.".into(),
                    images: Vec::new(),
                    skills: Vec::new(),
                    host: Default::default(),
                },
                Arc::new(Discard),
            )
            .await
            .unwrap();
        let turn = wait_for_turn_end(&service, session).await;
        assert_eq!(turn.permission_mode, expected);
    }
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The note survives a restart and reaches the next turn's request; the
/// session's permission mode does not survive it.
#[tokio::test]
async fn an_approved_denial_is_a_note_the_next_turn_sends_after_a_restart() {
    let arguments = serde_json::to_string(&json!({
        "cmd": approved_command(),
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null
    }))
    .unwrap();
    let denial = json!({
        "riskLevel": "high",
        "userAuthorization": "absent",
        "outcome": "deny",
        "rationale": "The user did not ask for this command."
    })
    .to_string();
    let server = FakeServer::start(vec![
        (tool_events("exec_command", &arguments), Duration::ZERO),
        (assistant_events(&denial), Duration::ZERO),
        (assistant_events("Not run."), Duration::ZERO),
        (assistant_events("Noted."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config) = service_with_config("approved-denial", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .set_session_permission_mode(session.id, AgentPermissionMode::AutoReview)
        .unwrap();
    let outcome = service
        .start_turn(session.id, "Run the command.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Not run.");
    let review_id = service
        .events_since(session.id, None)
        .unwrap()
        .events
        .into_iter()
        .find_map(|event| match event.kind {
            AgentEventKind::PermissionReviewCompleted {
                review_id,
                status: hanafubuki_agent_kernel::PermissionReviewStatus::Denied,
                ..
            } => Some(review_id),
            _ => None,
        })
        .expect("the command was denied");

    service
        .approve_denied_action(session.id, &review_id)
        .unwrap();
    assert_eq!(server.requests().len(), 3, "the approval starts no turn");
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
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
    assert_eq!(restarted.session_permission_mode(session.id), Ok(None));
    let outcome = restarted
        .start_turn(session.id, "Thanks.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Noted.");
    let request = String::from_utf8(server.requests()[3].body.clone()).unwrap();
    assert!(request.contains("provider kernel; hook approved-denied-action"));
    assert!(request.contains(
        "The user has manually approved a specific action that automatic permission review previously denied."
    ));
    assert!(restarted
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

async fn wait_for_turn_end(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> hanafubuki_agent_kernel::AgentTurnSnapshot {
    let deadline = Instant::now() + TEST_WAIT;
    loop {
        if let Some(turn) = service
            .snapshot(session_id)
            .unwrap()
            .turn
            .filter(|turn| turn.phase.is_terminal())
        {
            return turn;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the turn");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
