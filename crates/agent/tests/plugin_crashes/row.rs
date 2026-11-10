//! A panic of the `plugins` row itself: the row is `Failed` until its
//! restart, its guests stop with it and start again with it, and the
//! plugins' content, which other rows load, is not touched.

use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::TEST_WAIT;
use hanafubuki_agent::{AgentPluginState, AgentPluginStatus, McpConnectionPhase};

use crate::fixture::{answer, board, builtin, call, content_plugin, running, sandboxed, Agent};
use crate::mcp_server::starts;
use crate::watchdog::step;

fn row(agent: &Agent) -> AgentPluginStatus {
    agent
        .service
        .plugin_status()
        .into_iter()
        .find(|plugin| plugin.name == "plugins")
        .unwrap()
}

async fn wait_row(agent: &Agent, what: &str, done: impl Fn(&AgentPluginStatus) -> bool) {
    step(format!("wait for the row's {what}"));
    let deadline = Instant::now() + TEST_WAIT;
    loop {
        let status = row(agent);
        if done(&status) {
            return;
        }
        assert!(Instant::now() < deadline, "no {what}: {status:#?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn mcp_ready(agent: &Agent) -> bool {
    let snapshot = agent.service.mcp_snapshot().await.unwrap();
    snapshot.plugin_servers.iter().any(|server| {
        server.server.id.as_str() == "crashing" && server.status.phase == McpConnectionPhase::Ready
    })
}

pub async fn a_panic_of_the_plugins_row_stops_the_guests_and_spares_the_content() {
    let guests = sandboxed();
    let turn = |text: &str| {
        if guests {
            [call("demo", "echo"), answer(text)]
        } else {
            [builtin("skills_list"), answer(text)]
        }
    };
    let script = [turn("Before the panic."), turn("After the panic.")].concat();
    let agent = Agent::start("crashes-row", script).await;
    let state = agent.paths.app_data_dir.join("mcp-state");
    let source = agent.source("content");
    content_plugin(&source, "content");
    crate::content::crashing_server(&source, &state);
    agent.install(&source).await;
    crate::content::crashing_ready(&agent, &state, 1).await;
    let before = if guests {
        Some(agent.install_guest("demo", "run").await)
    } else {
        None
    };
    let outcome = agent.turn("Before the panic.").await;
    assert_eq!(outcome.final_answer, "Before the panic.");

    agent.service.panic_in_plugin("plugins");
    wait_row(&agent, "failure", |status| {
        status.state == AgentPluginState::Failed
            && status
                .error
                .as_deref()
                .is_some_and(|error| error.contains("a test made the plugin plugins panic"))
    })
    .await;
    // The content is other rows': the Skill is listed and the MCP server
    // runs on in the same process.
    let skills: Vec<String> = agent
        .service
        .list_skills()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.id.as_str().to_string())
        .collect();
    assert!(
        skills.contains(&"plugin:content@local:search".to_string()),
        "{skills:?}"
    );
    assert!(mcp_ready(&agent).await);

    wait_row(&agent, "restart", |status| {
        status.state == AgentPluginState::Active
    })
    .await;
    assert_eq!(
        row(&agent)
            .restart
            .map(|restart| restart.restarts_in_window),
        Some(1)
    );
    if let Some(before) = &before {
        // The guest stopped with the row and started again with it: a new
        // process, not a fault of its own.
        let back = agent.restarted("demo@local", before).await;
        assert!(running(&back));
        assert_eq!(back.restarts, 0, "{back:#?}");
        #[cfg(target_os = "linux")]
        assert!(crate::fixture::gone(before.process_id.unwrap()));
    }
    assert!(mcp_ready(&agent).await);
    assert_eq!(starts(&state), 1);

    let outcome = agent.turn("After the panic.").await;
    assert_eq!(outcome.final_answer, "After the panic.");
    let first = agent.request(2);
    assert!(first.contains("content:search"), "{first}");
    assert!(first.contains("(`content@local`)"), "{first}");
    if guests {
        assert!(first.contains(&board("demo")), "{first}");
        assert!(agent.last_request().contains(r#"\"by\":\"demo\""#));
    }
    agent.finish().await;
}
