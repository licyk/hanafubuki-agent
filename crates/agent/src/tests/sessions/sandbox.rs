//! The sandbox a session's commands run in. This test binary never calls
//! `sandbox::helper_dispatch`, so it has no sandbox but on macOS, whose
//! Seatbelt needs no helper (the test configuration is full access,
//! `configure_test_provider`): a command that asks for one fails closed
//! here, which is what the second test checks (the sandboxes themselves
//! are `hanafubuki-agent-sandbox`'s `tests/linux.rs` and `tests/macos.rs`,
//! and the command line's `tests/exec.rs`).

use super::*;
use hanafubuki_agent_sandbox::{platform_default_mode, SandboxSettings};

#[tokio::test]
async fn a_turn_takes_the_session_sandbox_over_the_configured_one() {
    let (service, paths, _) = service_with_config("session-sandbox", |_, config| {
        configure_test_provider(config, "http://127.0.0.1:9/v1", None);
        config
            .write_settings(AgentSettingsPatch {
                // Unset again: the platform's default.
                sandbox_mode: Some(None),
                sandbox_workspace_write: Some(AgentSandboxWorkspaceWrite {
                    writable_roots: vec![std::env::temp_dir().join("sandbox-data")],
                    network_access: true,
                }),
                ..AgentSettingsPatch::default()
            })
            .unwrap();
    });
    let chosen = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let other = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let configured = SandboxSettings {
        mode: platform_default_mode(),
        writable_roots: vec![std::env::temp_dir().join("sandbox-data")],
        network_access: true,
    };
    let driver = service.test_session_driver();
    assert_eq!(driver.turn_sandbox(chosen.id).await.unwrap(), configured);

    service
        .set_session_sandbox_mode(chosen.id, AgentSandboxMode::ReadOnly)
        .unwrap();

    assert_eq!(
        service.session_sandbox_mode(chosen.id),
        Ok(Some(AgentSandboxMode::ReadOnly))
    );
    assert_eq!(service.session_sandbox_mode(other.id), Ok(None));
    let turn = driver
        .collect_turn_extensions(chosen.id, AgentSessionMode::Default)
        .await
        .unwrap();
    assert_eq!(
        turn.get::<SandboxSettings>(),
        Some(&SandboxSettings {
            mode: AgentSandboxMode::ReadOnly,
            ..configured.clone()
        })
    );
    assert_eq!(driver.turn_sandbox(other.id).await.unwrap(), configured);
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Even a full-access permission mode, which asks no one, runs nothing
/// when the sandbox the command needs is unavailable: the model reads why.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn a_command_whose_sandbox_is_unavailable_is_refused_before_it_runs() {
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
    let server = FakeServer::start(vec![
        (tool_events("exec_command", &arguments), Duration::ZERO),
        (assistant_events("Not run."), Duration::ZERO),
        (assistant_events("Title"), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("sandbox-unavailable", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .set_session_permission_mode(
            session.id,
            hanafubuki_agent_approval::AgentPermissionMode::FullAccess,
        )
        .unwrap();
    service
        .set_session_sandbox_mode(session.id, AgentSandboxMode::WorkspaceWrite)
        .unwrap();
    assert!(matches!(
        service.sandbox_status(),
        hanafubuki_agent_sandbox::SandboxStatus::Unavailable { .. }
    ));

    let outcome = service
        .start_turn(session.id, "Run the command.".to_string().into())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Not run.");
    let request = String::from_utf8(server.requests()[1].body.clone()).unwrap();
    assert!(
        request.contains("needs the workspace-write sandbox, which is unavailable"),
        "{request}"
    );
    assert!(!request.contains("approved-agent-command\\n"), "{request}");
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
