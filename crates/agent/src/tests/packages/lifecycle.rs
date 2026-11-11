//! A Claude Code plugin's Skill and hook, over the scripted model: once the
//! plugin is on, its Skill is listed and selected by `$demo:search`, the
//! model is told of the plugin, and its trusted hook denies a call; a change
//! to the package stops the hook until it is trusted again; once the plugin
//! is off, neither is there.

use hanafubuki_agent_kernel::AgentSessionMode;

use super::*;
use crate::{HookSource, HookTrust};

fn request_text(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

fn denials(request: &str) -> usize {
    request.matches("a PreToolUse hook denied the call").count()
}

// The hooks run on the blocking pool, which must finish while `shutdown`
// blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_claude_code_plugins_skill_and_hook_work_once_on_and_stop_once_off() {
    let server = FakeServer::start(
        [
            tool_events("skills_list", "{}"),
            assistant_events("Denied."),
            tool_events("skills_list", "{}"),
            assistant_events("Listed after the change."),
            tool_events("skills_list", "{}"),
            assistant_events("Listed."),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths, config) = service_with_config("packages-lifecycle", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let source = claude_plugin(
        &paths,
        "demo",
        json!({}),
        Some(json!({ "PreToolUse": [{
            "matcher": "skills_list",
            "hooks": [{
                "type": "command",
                "command": "cat >/dev/null; echo \"denied by $CLAUDE_PLUGIN_ROOT\" >&2; exit 2",
                "timeout": 30,
            }],
        }]})),
    );
    let root = install(&paths, &source, "demo@local");
    set_enabled(&config, "demo@local", false);
    service.reload_packages().await.unwrap();
    assert!(plugin_skill_ids(&service).is_empty());

    set_enabled(&config, "demo@local", true);
    service.reload_packages().await.unwrap();

    assert_eq!(plugin_skill_ids(&service), ["plugin:demo@local:search"]);
    let hooks = service.hooks_list().await.unwrap();
    let [hook] = hooks.hooks.as_slice() else {
        panic!("one plugin hook: {:?}", hooks.hooks);
    };
    assert_eq!(hook.trust, HookTrust::Untrusted);
    assert!(matches!(&hook.source, HookSource::Plugin { id, .. } if id == "demo@local"));
    service.trust_hooks(vec![hook.key.clone()]).await.unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();

    let outcome = service
        .start_turn(
            session.id,
            "Use $demo:search to list the Skills.".to_string().into(),
        )
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Denied.");
    let first = request_text(&server, 0);
    assert!(first.contains("<name>demo:search</name>"), "{first}");
    assert!(first.contains("Search the records with the plugin's tools."));
    assert!(
        first.contains("- `demo` (`demo@local`): The demo plugin. Skills: prefixed with `demo:`.")
    );
    assert!(request_text(&server, 1).contains(&format!(
        "a PreToolUse hook denied the call: denied by {}",
        root.display()
    )));
    let status = package(&service, "demo@local").await;
    assert_eq!(status.content.skills, ["plugin:demo@local:search"]);
    assert_eq!(status.content.hooks, std::slice::from_ref(&hook.key));

    write(&root.join("NOTES.md"), "changed after it was trusted\n");
    assert_eq!(
        service.hooks_list().await.unwrap().hooks[0].trust,
        HookTrust::Modified
    );
    let outcome = service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Listed after the change.");
    // Only the first turn's denial is in the history.
    assert_eq!(denials(&request_text(&server, 3)), 1);

    set_enabled(&config, "demo@local", false);
    service.reload_packages().await.unwrap();

    assert!(plugin_skill_ids(&service).is_empty());
    assert!(service.hooks_list().await.unwrap().hooks.is_empty());
    let outcome = service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Listed.");
    assert!(!request_text(&server, 4).contains("plugins_instructions"));
    assert_eq!(denials(&request_text(&server, 5)), 1);
    finish(service, paths).await;
}
