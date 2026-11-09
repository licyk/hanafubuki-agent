//! The facade's feature calls: a recap and a title suggestion from the
//! plugin rows that provide them, `FeatureUnavailable` while a row is off,
//! and a panicking feature service contained as its row's fault.

use hanafubuki_agent_kernel::{AgentError, AgentSessionMode};
use kardia_core::{Context, PluginDef};

use super::host::{AgentPluginGroup, AgentPluginRow, AgentPluginState, AgentPluginStatus};
use super::*;

const RECAP: &str =
    r#"{"summary":"Fixed the login timeout; not deployed yet.","next_action":"Deploy the fix?"}"#;

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn status(service: &AgentApplicationService, name: &str) -> AgentPluginStatus {
    service
        .plugin_status()
        .into_iter()
        .find(|status| status.name == name)
        .unwrap_or_else(|| panic!("no plugin {name}"))
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A started service on `server` whose session has one finished turn. The
/// turn is programmatic, so no automatic title races the feature call for
/// the server's answers.
async fn service_with_a_turn(
    name: &str,
    server: &FakeServer,
) -> (AgentApplicationService, TestPaths, AgentSessionId) {
    let (service, paths) = configured_service(name, &server.base_url);
    service.start_plugins().await;
    let session_id = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap()
        .id;
    service
        .start_turn(session_id, "Fix the login timeout".to_string().into())
        .await
        .unwrap();
    (service, paths, session_id)
}

/// The recap reads the session's exchange and returns the model's answer.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_recap_comes_from_the_sessions_model() {
    let server = FakeServer::start(vec![
        (assistant_events("Fixed the timeout."), Duration::ZERO),
        (assistant_events(RECAP), Duration::ZERO),
    ])
    .await;
    let (service, paths, session_id) = service_with_a_turn("feature-recap", &server).await;

    let recap = service.recap_session(session_id).await.unwrap();

    assert_eq!(
        recap,
        AgentRecap {
            summary: "Fixed the login timeout; not deployed yet.".to_string(),
            next_action: Some("Deploy the fix?".to_string()),
        }
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let body = String::from_utf8_lossy(&requests[1].body);
    assert!(body.contains("Write a brief catch-up"), "{body}");
    assert!(body.contains("User: Fix the login timeout"), "{body}");
    assert!(body.contains("Assistant: Fixed the timeout."), "{body}");
    // Nothing is committed: the session's history is the turn's.
    let open = service.open_session(session_id, 50).unwrap();
    assert!(!format!("{open:?}").contains("not deployed yet"));
    finish(service, paths);
}

/// A title suggestion comes from the `titles` row's service.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_title_suggestion_comes_from_the_sessions_model() {
    let server = FakeServer::start(vec![
        (assistant_events("Fixed the timeout."), Duration::ZERO),
        (
            assistant_events(r#"{"title":"Fix   login timeout"}"#),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths, session_id) = service_with_a_turn("feature-title", &server).await;

    let title = service.suggest_session_title(session_id).await.unwrap();

    assert_eq!(title, "Fix login timeout");
    let body = String::from_utf8_lossy(&server.requests()[1].body).to_string();
    assert!(body.contains("conversation excerpted"), "{body}");
    assert!(body.contains("User: Fix the login timeout"), "{body}");
    assert_eq!(service.snapshot(session_id).unwrap().title, None);
    finish(service, paths);
}

/// With the `recap` plugin off a recap is unavailable, saying why, and the
/// rest of the Agent works on: a title suggestion and a turn.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_recap_plugin_off_only_recaps_are_unavailable() {
    let server = FakeServer::start(vec![
        (assistant_events("Fixed the timeout."), Duration::ZERO),
        (assistant_events(r#"{"title":"Fix login"}"#), Duration::ZERO),
        (assistant_events("Again."), Duration::ZERO),
    ])
    .await;
    let (service, paths, session_id) = service_with_a_turn("feature-recap-off", &server).await;
    service
        .set_plugin_enabled("recap".into(), false)
        .await
        .unwrap();
    wait_until("the recap plugin is off", || {
        status(&service, "recap").state == AgentPluginState::Disabled
    })
    .await;

    let error = service.recap_session(session_id).await.unwrap_err();

    assert_eq!(
        error,
        AgentError::FeatureUnavailable {
            feature: "recap".to_string(),
            reason: "the recap Agent plugin is unavailable: it is turned off".to_string(),
        }
    );
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        service.suggest_session_title(session_id).await.unwrap(),
        "Fix login"
    );
    service
        .start_turn(session_id, "Once more".to_string().into())
        .await
        .unwrap();
    assert_eq!(server.requests().len(), 3);
    finish(service, paths);
}

/// What the `feature-test` row provides.
struct FeatureProbe;

fn feature_probe_plugin() -> PluginDef {
    PluginDef::new(|ctx: Context, _| async move {
        ctx.provide_arc_as("feature-test", Arc::new(FeatureProbe))?;
        Ok(())
    })
    .name("feature-test")
    .provides(["feature-test"])
}

/// A panic in a feature service's call is that row's fault: the call is
/// unavailable, the row is stopped as failed while every other plugin runs
/// on, and a retry serves calls again.
#[tokio::test]
async fn a_panicking_feature_call_is_its_rows_fault() {
    let paths = test_paths("feature-panics");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, "http://127.0.0.1:9", None);
    let service = AgentApplicationService::new_with_test_plugins(
        &paths,
        config,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        vec![AgentPluginRow::unconfigured(
            AgentPluginGroup::Capability,
            feature_probe_plugin(),
        )],
    )
    .unwrap();
    let call = |panics: bool| {
        service.feature("feature-test", move |_: Arc<FeatureProbe>| async move {
            if panics {
                panic!("the feature service panicked");
            }
            Ok("served")
        })
    };

    assert_eq!(call(false).await.unwrap(), "served");
    let error = call(true).await.unwrap_err();

    assert!(
        matches!(&error, AgentError::FeatureUnavailable { feature, .. } if feature == "feature-test"),
        "{error:?}"
    );
    wait_until("the row fails", || {
        status(&service, "feature-test").state == AgentPluginState::Failed
    })
    .await;
    assert_eq!(
        status(&service, "feature-test").error.unwrap(),
        "panicked in feature call: the feature service panicked"
    );
    for status in service.plugin_status() {
        if status.name != "feature-test" && status.enabled {
            assert_eq!(status.state, AgentPluginState::Active, "{}", status.name);
        }
    }
    assert!(matches!(
        call(false).await.unwrap_err(),
        AgentError::FeatureUnavailable { .. }
    ));

    service.retry_plugin("feature-test").await.unwrap();
    assert_eq!(call(false).await.unwrap(), "served");
    finish(service, paths);
}
