//! The failures of a plugin's content: a bad manifest or component costs
//! that package (or component) alone, a crashed MCP server of a plugin is
//! reconnected while the turn goes on, and a failing hook has no effect.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::TEST_WAIT;
use hanafubuki_agent::McpConnectionPhase;
use serde_json::json;

use crate::fixture::{
    answer, board, builtin, call, code_plugin, content_plugin, sandboxed, search, write, Agent,
};
use crate::mcp_server::{starts, MCP_ARG};
use crate::watchdog::step;

pub async fn a_bad_manifest_or_component_costs_only_its_package() {
    let guests = sandboxed();
    let script = if guests {
        vec![call("demo", "echo"), answer("The siblings work.")]
    } else {
        vec![builtin("skills_list"), answer("The siblings work.")]
    };
    let agent = Agent::start("crashes-bad-manifest", script).await;
    let good = agent.source("good");
    content_plugin(&good, "good");
    agent.install(&good).await;
    // A package whose manifest is broken once installed.
    let broken = agent.source("broken");
    content_plugin(&broken, "broken");
    let installed = agent.install(&broken).await.root;
    write(
        &installed.join(".claude-plugin/plugin.json"),
        "{ this is not JSON",
    );
    // A package whose MCP servers and hooks cannot be read: its Skill
    // still loads.
    let halfbroken = agent.source("halfbroken");
    content_plugin(&halfbroken, "halfbroken");
    write(&halfbroken.join(".mcp.json"), "[ not an object");
    write(&halfbroken.join("hooks/hooks.json"), "{ not JSON");
    agent.install(&halfbroken).await;
    if guests {
        agent.install_guest("demo", "run").await;
        // A code package whose program is missing: its guest fails to start.
        let missing = agent.source("missing");
        code_plugin(&missing, "missing", "run");
        let manifest = missing.join(".hanafubuki-plugin/plugin.json");
        let text = std::fs::read_to_string(&manifest).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["runtime"] = json!({ "type": "process", "command": ["./no-such-guest"] });
        write(&manifest, &value.to_string());
        let id = agent.install(&missing).await.package.id;
        agent
            .service
            .grant_package_permissions(&id, Default::default())
            .await
            .unwrap();
        let failed = agent
            .wait_guest(&id, "its failed start", |status| {
                status.state == hanafubuki_agent::GuestState::Failed
            })
            .await;
        assert!(failed.error.is_some(), "{failed:#?}");
    }
    agent.service.reload_packages().await.unwrap();

    let broken = agent.package("broken@local").await;
    assert!(!broken.issues.is_empty(), "{broken:#?}");
    let halfbroken = agent.package("halfbroken@local").await;
    assert!(
        halfbroken
            .issues
            .iter()
            .any(|issue| issue.contains(".mcp.json cannot be read")),
        "{halfbroken:#?}"
    );
    // The hooks engine reports a hooks file it cannot read.
    let hooks = agent.service.hooks_list().await.unwrap();
    assert!(hooks.hooks.is_empty(), "{hooks:#?}");
    assert!(
        hooks
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("hooks.json cannot be read")),
        "{hooks:#?}"
    );
    assert_eq!(
        halfbroken.content.skills,
        ["plugin:halfbroken@local:search"]
    );
    let good = agent.package("good@local").await;
    assert!(good.issues.is_empty(), "{good:#?}");

    let outcome = agent.turn("Use the plugins.").await;
    assert_eq!(outcome.final_answer, "The siblings work.");
    let first = agent.request(0);
    assert!(first.contains("good:search"), "{first}");
    assert!(first.contains("halfbroken:search"), "{first}");
    if guests {
        assert!(first.contains(&board("demo")), "{first}");
        assert!(agent.last_request().contains(r#"\"by\":\"demo\""#));
    }
    agent.finish().await;
}

/// The `.mcp.json` of a plugin whose stdio server `crashing` is this
/// binary, counting its starts in `state`.
pub fn crashing_server(source: &Path, state: &Path) {
    std::fs::create_dir_all(state).unwrap();
    let program = std::env::current_exe().unwrap();
    write(
        &source.join(".mcp.json"),
        &json!({ "mcpServers": { "crashing": {
            "command": program,
            "args": [MCP_ARG, state],
        }}})
        .to_string(),
    );
}

/// Waits until the plugin's MCP server `crashing` is ready, started
/// `count` times.
pub async fn crashing_ready(agent: &Agent, state: &Path, count: u32) {
    step(format!("wait for the MCP server's start {count}"));
    let deadline = Instant::now() + TEST_WAIT;
    loop {
        let snapshot = agent.service.mcp_snapshot().await.unwrap();
        let ready = snapshot.plugin_servers.iter().any(|server| {
            server.server.id.as_str() == "crashing"
                && server.status.phase == McpConnectionPhase::Ready
        });
        if ready && starts(state) == count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the MCP server never got ready ({} starts): {:#?}",
            starts(state),
            snapshot.plugin_servers
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Where the content plugin `crashing`'s server keeps its state.
fn mcp_state(agent: &Agent) -> PathBuf {
    agent.paths.app_data_dir.join("mcp-state")
}

pub async fn a_plugins_mcp_server_that_crashes_is_reconnected_and_the_turn_goes_on() {
    let agent = Agent::start(
        "crashes-mcp",
        vec![
            search("die"),
            call("mcp__crashing", "die"),
            answer("The server crashed; going on."),
            search("echo records"),
            call("mcp__crashing", "echo"),
            answer("The server is back."),
        ],
    )
    .await;
    let state = mcp_state(&agent);
    let source = agent.source("crashing");
    content_plugin(&source, "crashing");
    crashing_server(&source, &state);
    agent.install(&source).await;
    let sibling = agent.source("good");
    content_plugin(&sibling, "good");
    agent.install(&sibling).await;
    crashing_ready(&agent, &state, 1).await;

    let outcome = agent.turn("Crash the server.").await;
    assert_eq!(outcome.final_answer, "The server crashed; going on.");
    let after_crash = agent.request(2);
    assert!(
        !after_crash.contains("the crashing server echoed"),
        "{after_crash}"
    );

    // The supervision connects it again, after its first delay.
    crashing_ready(&agent, &state, 2).await;
    let outcome = agent.turn("Use the server again.").await;
    assert_eq!(outcome.final_answer, "The server is back.");
    let after = agent.last_request();
    assert!(after.contains("the crashing server echoed"), "{after}");
    assert!(after.contains("good:search"), "{after}");
    agent.finish().await;
}

/// The failing hooks run on Unix shells here (`sh -c`); the hooks
/// themselves are checked on every platform by the hooks crate.
pub async fn a_plugins_failing_hooks_have_no_effect() {
    if !cfg!(unix) {
        eprintln!("  skipped: the failing hooks are shell commands for `sh`");
        return;
    }
    let agent = Agent::start(
        "crashes-hooks",
        vec![
            builtin("skills_list"),
            answer("Listed despite the hooks."),
            builtin("skills_list"),
            answer("Listed again."),
        ],
    )
    .await;
    let markers = agent.paths.app_data_dir.join("markers");
    std::fs::create_dir_all(&markers).unwrap();
    let pre = markers.join("pre");
    let post = markers.join("post");
    let source = agent.source("hooky");
    content_plugin(&source, "hooky");
    // Only the hook that hangs times out; one that touches its marker has
    // the time a loaded runner takes to start its shell.
    let command = |text: String, timeout: u64| json!({ "type": "command", "command": text, "timeout": timeout });
    write(
        &source.join("hooks/hooks.json"),
        &json!({ "hooks": {
            "PreToolUse": [{ "matcher": "skills_list", "hooks": [
                command(format!("cat >/dev/null; touch '{}'; exit 1", pre.display()), 30),
                command("hanafubuki-no-such-hook-program".to_string(), 30),
                command("sleep 30".to_string(), 1),
            ]}],
            "PostToolUse": [{ "matcher": "skills_list", "hooks": [
                command(format!("cat >/dev/null; touch '{}'; echo 'not JSON {{'", post.display()), 30),
            ]}],
        }})
        .to_string(),
    );
    agent.install(&source).await;
    let hooks = agent.service.hooks_list().await.unwrap().hooks;
    assert_eq!(hooks.len(), 4, "{hooks:#?}");
    agent
        .service
        .trust_hooks(hooks.into_iter().map(|hook| hook.key).collect())
        .await
        .unwrap();

    let outcome = agent.turn("List the Skills.").await;
    assert_eq!(outcome.final_answer, "Listed despite the hooks.");
    assert!(pre.exists(), "the failing PreToolUse hook never ran");
    assert!(post.exists(), "the failing PostToolUse hook never ran");
    let after = agent.request(1);
    assert!(after.contains("hooky:search"), "{after}");
    assert!(!after.contains("denied"), "{after}");

    let outcome = agent.turn("List them again.").await;
    assert_eq!(outcome.final_answer, "Listed again.");
    assert!(agent.last_request().contains("hooky:search"));
    agent.finish().await;
}
