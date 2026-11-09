//! The user's hooks end to end, over the scripted model: a trusted
//! PreToolUse hook denies a call, a PostToolUse hook adds context after the
//! step's outputs, a Stop hook keeps a turn going once, a hook past its
//! timeout has no say, an untrusted hook never runs, and with the `hooks`
//! row off nothing runs and the hook calls are unavailable.

use hanafubuki_agent_kernel::{
    AgentError, AgentHookEvent, AgentHookOutcome, AgentSessionMode, ConversationItem,
};

use super::host::AgentPluginState;
use super::*;
use crate::{AgentHistorySortDirection, AgentTranscriptPageRequest, HookTrust};

/// A service whose user configuration has `hooks` as its `hooks.json`,
/// over a model that answers with `responses`.
async fn hooked_service(
    name: &str,
    hooks: serde_json::Value,
    responses: Vec<Vec<u8>>,
) -> (AgentApplicationService, TestPaths, FakeServer) {
    let server = FakeServer::start(
        responses
            .into_iter()
            .map(|response| (response, Duration::ZERO))
            .collect(),
    )
    .await;
    let (service, paths) = service_with(name, |paths, config| {
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(
            paths.config_dir.join("hooks.json"),
            serde_json::to_vec(&json!({ "hooks": hooks })).unwrap(),
        )
        .unwrap();
        configure_test_provider(config, &server.base_url, None);
    });
    (service, paths, server)
}

async fn trust_every_hook(service: &AgentApplicationService) {
    let listed = service.hooks_list().await.unwrap();
    assert!(listed
        .hooks
        .iter()
        .all(|hook| hook.trust == HookTrust::Untrusted));
    let keys = listed.hooks.into_iter().map(|hook| hook.key).collect();
    service.trust_hooks(keys).await.unwrap();
}

fn command(command: &str) -> serde_json::Value {
    json!({ "type": "command", "command": command, "timeout": 30 })
}

fn hook_outcomes(
    service: &AgentApplicationService,
    session: AgentSessionId,
) -> Vec<(AgentHookEvent, AgentHookOutcome)> {
    service
        .events_since(session, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::HookCompleted { event, outcome, .. } => Some((event, outcome)),
            _ => None,
        })
        .collect()
}

fn request_text(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

async fn finish(service: AgentApplicationService, paths: TestPaths) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

// The hooks run on the blocking pool, which must finish while `shutdown`
// blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_trusted_pre_tool_use_hook_denies_the_call_with_its_reason() {
    let (service, paths, server) = hooked_service(
        "hooks-pre-tool-use",
        json!({ "PreToolUse": [{
            "matcher": "skills_list",
            "hooks": [command("cat >/dev/null; echo 'not in this repository' >&2; exit 2")],
        }]}),
        vec![
            tool_events("skills_list", "{}"),
            assistant_events("Denied."),
        ],
    )
    .await;
    trust_every_hook(&service).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();

    let outcome = service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Denied.");
    assert!(request_text(&server, 1)
        .contains("a PreToolUse hook denied the call: not in this repository"));
    assert_eq!(
        hook_outcomes(&service, session.id),
        [(AgentHookEvent::PreToolUse, AgentHookOutcome::Denied)]
    );
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_post_tool_use_hook_adds_context_after_the_steps_outputs() {
    let output = json!({ "hookSpecificOutput": {
        "hookEventName": "PostToolUse",
        "additionalContext": "the Skill catalog was audited",
    }});
    let (service, paths, server) = hooked_service(
        "hooks-post-tool-use",
        json!({ "PostToolUse": [{
            "matcher": "*",
            "hooks": [command(&format!("cat >/dev/null; echo '{output}'"))],
        }]}),
        vec![
            tool_events("skills_list", "{}"),
            assistant_events("Listed."),
        ],
    )
    .await;
    trust_every_hook(&service).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();

    service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();

    assert!(request_text(&server, 1).contains("the Skill catalog was audited"));
    let page = service
        .read_transcript_page(AgentTranscriptPageRequest {
            session_id: session.id,
            page_size: 50,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    let kinds = page
        .items
        .iter()
        .filter_map(|item| match &item.content {
            ConversationItem::ToolOutput { .. } => Some("output".to_string()),
            ConversationItem::ContinuationContext {
                provider_id,
                content,
                ..
            } => Some(format!("{}: {content}", provider_id.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        ["output", "hooks: the Skill catalog was audited"],
        "the context follows the step's outputs"
    );
    assert_eq!(
        hook_outcomes(&service, session.id),
        [(AgentHookEvent::PostToolUse, AgentHookOutcome::AddedContext)]
    );
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_hook_keeps_the_turn_going_once() {
    let (service, paths, server) = hooked_service(
        "hooks-stop",
        json!({ "Stop": [{ "hooks": [command(
            "if grep -q '\"stop_hook_active\":false'; then \
             echo '{\"decision\":\"block\",\"reason\":\"check the tests first\"}'; fi"
        )] }]}),
        vec![assistant_events("Done."), assistant_events("Tests pass.")],
    )
    .await;
    trust_every_hook(&service).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();

    let outcome = service
        .start_turn(session.id, "Finish up.".to_string().into())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Tests pass.");
    assert_eq!(server.requests().len(), 2);
    assert!(request_text(&server, 1).contains("check the tests first"));
    let continuations = service
        .events_since(session.id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ContinuationHookCompleted { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        continuations,
        [
            hanafubuki_agent_kernel::AgentContinuationHookOutcome::Continued,
            hanafubuki_agent_kernel::AgentContinuationHookOutcome::Passed
        ]
    );
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hook_past_its_timeout_has_no_say() {
    // Far past its 1 s timeout: a turn that ends before it did not wait for
    // the hook, however slow the machine.
    const HOOK_RUNS_FOR: Duration = Duration::from_secs(60);
    let (service, paths, _server) = hooked_service(
        "hooks-timeout",
        json!({ "PreToolUse": [{ "hooks": [{
            "type": "command",
            "command": format!(
                "sleep {}; echo 'too late' >&2; exit 2",
                HOOK_RUNS_FOR.as_secs()
            ),
            "timeout": 1,
        }]}]}),
        vec![
            tool_events("skills_list", "{}"),
            assistant_events("Listed."),
        ],
    )
    .await;
    trust_every_hook(&service).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let started = Instant::now();

    service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();

    assert!(
        started.elapsed() < HOOK_RUNS_FOR,
        "the turn waited for the hook: {:?}",
        started.elapsed()
    );
    assert_eq!(
        hook_outcomes(&service, session.id),
        [(AgentHookEvent::PreToolUse, AgentHookOutcome::Failed)],
        "a failed hook does not deny the call"
    );
    let calls = service
        .events_since(session.id, None)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| {
            matches!(
                event.kind,
                AgentEventKind::ToolCompleted {
                    is_error: false,
                    ..
                }
            )
        })
        .count();
    assert_eq!(calls, 1, "the call ran");
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_untrusted_or_modified_hook_never_runs() {
    let deny = json!({ "PreToolUse": [{
        "hooks": [command("echo 'hook-marker-denied' >&2; exit 2")],
    }]});
    let (service, paths, server) = hooked_service(
        "hooks-untrusted",
        deny.clone(),
        vec![
            tool_events("skills_list", "{}"),
            assistant_events("Listed."),
            tool_events("skills_list", "{}"),
            assistant_events("Listed again."),
        ],
    )
    .await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();

    service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();
    trust_every_hook(&service).await;
    std::fs::write(
        paths.config_dir.join("hooks.json"),
        serde_json::to_vec(&json!({ "hooks": { "PreToolUse": [{
            "hooks": [command("echo 'hook-marker-changed' >&2; exit 2")],
        }]}}))
        .unwrap(),
    )
    .unwrap();
    service
        .start_turn(session.id, "List them again.".to_string().into())
        .await
        .unwrap();

    assert!(hook_outcomes(&service, session.id).is_empty());
    assert!(!request_text(&server, 1).contains("hook-marker-denied"));
    assert!(!request_text(&server, 3).contains("hook-marker-changed"));
    let listed = service.hooks_list().await.unwrap();
    assert_eq!(listed.hooks.len(), 1);
    assert_eq!(listed.hooks[0].trust, HookTrust::Modified);
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_plugin_off_no_hook_runs_and_the_calls_are_unavailable() {
    let (service, paths, server) = hooked_service(
        "hooks-off",
        json!({ "PreToolUse": [{ "hooks": [command("echo 'hook-marker-denied' >&2; exit 2")] }]}),
        vec![
            tool_events("skills_list", "{}"),
            assistant_events("Listed."),
        ],
    )
    .await;
    trust_every_hook(&service).await;
    let key = service.hooks_list().await.unwrap().hooks[0].key.clone();
    service.set_hook_enabled(key.clone(), false).await.unwrap();
    assert!(!service.hooks_list().await.unwrap().hooks[0].enabled);
    service.set_hook_enabled(key, true).await.unwrap();

    service
        .set_plugin_enabled("hooks".into(), false)
        .await
        .unwrap();
    let deadline = Instant::now() + TEST_WAIT;
    while service
        .plugin_status()
        .into_iter()
        .any(|status| status.name == "hooks" && status.state != AgentPluginState::Disabled)
    {
        assert!(Instant::now() < deadline, "the hooks plugin stayed on");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();

    assert!(hook_outcomes(&service, session.id).is_empty());
    assert!(!request_text(&server, 1).contains("hook-marker-denied"));
    assert_eq!(
        service.hooks_list().await.unwrap_err(),
        AgentError::FeatureUnavailable {
            feature: "hooks".to_string(),
            reason: "the hooks Agent plugin is unavailable: it is turned off".to_string(),
        }
    );
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn session_start_and_user_prompt_submit_hooks_add_world_state() {
    let (service, paths, server) = hooked_service(
        "hooks-context",
        json!({
            "SessionStart": [{ "hooks": [command("cat >/dev/null; echo hook-session-start")] }],
            "UserPromptSubmit": [{ "hooks": [command(
                "grep -q 'the second message' && echo hook-second-prompt || echo hook-first-prompt"
            )] }],
        }),
        vec![assistant_events("One."), assistant_events("Two.")],
    )
    .await;
    trust_every_hook(&service).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();

    service
        .start_turn(session.id, "the first message".to_string().into())
        .await
        .unwrap();
    service
        .start_turn(session.id, "the second message".to_string().into())
        .await
        .unwrap();

    let first = request_text(&server, 0);
    assert!(first.contains("hook-session-start") && first.contains("hook-first-prompt"));
    let second = request_text(&server, 1);
    assert!(
        second.contains("hook-second-prompt"),
        "the latest prompt's context is shown"
    );
    assert!(second.contains("hook-session-start"));
    finish(service, paths).await;
}
