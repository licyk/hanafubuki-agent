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

/// A turn `start_turn` runs from text takes the session's own mode, else the
/// configured one; a mode in the input wins over both. Under a configured
/// `FullAccess` the command runs without asking, and `Manual`, the
/// session's or the input's, asks the user.
#[tokio::test(flavor = "multi_thread")]
async fn start_turn_runs_in_the_inputs_mode_else_the_sessions_else_the_configured_one() {
    use AgentPermissionMode::{FullAccess, Manual};
    let arguments = json!({
        "cmd": approved_command(),
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null
    })
    .to_string();
    let mut responses = Vec::new();
    for _ in 0..4 {
        responses.push((tool_events("exec_command", &arguments), Duration::ZERO));
        responses.push((assistant_events("Ran it."), Duration::ZERO));
    }
    let server = FakeServer::start(responses).await;
    let (service, paths, _) = service_with_config("start-turn-permission-mode", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .write_settings(crate::AgentSettingsPatch {
                permission_mode: Some(FullAccess),
                // No sandbox, so the command runs.
                sandbox_mode: Some(Some(AgentSandboxMode::FullAccess)),
                ..Default::default()
            })
            .unwrap();
    });
    let unset = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let chosen = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .set_session_permission_mode(chosen.id, Manual)
        .unwrap();

    // The session, the mode the input gives, the mode the turn runs in,
    // and whether the user is asked.
    for (session, given, expected, asks) in [
        (unset.id, None, FullAccess, false),
        (unset.id, Some(Manual), Manual, true),
        (chosen.id, None, Manual, true),
        (chosen.id, Some(FullAccess), FullAccess, false),
    ] {
        let input = crate::AgentTurnInput {
            permission_mode: given,
            .."Run it.".into()
        };
        let asked = turn_allowing_once(&service, session, input, asks).await;
        assert_eq!(asked, usize::from(asks), "{given:?}");
        let turn = service.snapshot(session).unwrap().turn.unwrap();
        assert_eq!(turn.permission_mode, expected);
        let ran = String::from_utf8(server.requests().last().unwrap().body.clone()).unwrap();
        assert!(ran.contains("approved-agent-command"), "{ran}");
    }
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Runs `input`'s turn on `session_id`, allowing once the approval it asks
/// for if it `asks`: how many approvals the turn asked for.
async fn turn_allowing_once(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    input: crate::AgentTurnInput,
    asks: bool,
) -> usize {
    let before = approvals(service, session_id).len();
    let running = service.clone();
    let turn = tokio::spawn(async move { running.start_turn(session_id, input).await });
    if asks {
        let deadline = Instant::now() + TEST_WAIT;
        let request = loop {
            if let Some(request) = approvals(service, session_id).into_iter().nth(before) {
                break request;
            }
            assert!(Instant::now() < deadline, "no approval asked for");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let identity = service.snapshot(session_id).unwrap().turn.unwrap();
        service
            .resolve_approval(
                session_id,
                identity.id,
                identity.generation,
                request.request_id,
                ApprovalUserDecision::AllowOnce,
            )
            .unwrap();
    }
    let outcome = tokio::time::timeout(TEST_WAIT, turn)
        .await
        .expect("the turn ends without another approval")
        .unwrap()
        .unwrap();
    assert_eq!(outcome.final_answer, "Ran it.");
    approvals(service, session_id).len() - before
}

fn approvals(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> Vec<hanafubuki_agent_approval::ApprovalRequest> {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ApprovalRequested { request } => Some(request),
            _ => None,
        })
        .collect()
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
