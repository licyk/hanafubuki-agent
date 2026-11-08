//! A stdio server whose process exits is `Failed` with the reason at once,
//! and the manager connects it again within its restart policy; a step that
//! still holds the dead connection's lease gets `CatalogChanged`.

use std::path::Path;

use hanafubuki_agent_host::RestartPolicy;

use super::*;

/// Counts its spawns in `<state>/spawns`; a spawn after the first exits at
/// once while `<state>/broken` exists; the `die` tool exits with code 3.
const DYING_SERVER: &str = r#"
import json, os, sys
state = sys.argv[1]
spawns = os.path.join(state, "spawns")
count = int(open(spawns).read()) + 1 if os.path.exists(spawns) else 1
open(spawns, "w").write(str(count))
if count > 1 and os.path.exists(os.path.join(state, "broken")):
    sys.exit(1)
schema = {"type": "object", "properties": {}, "additionalProperties": False}
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "initialize":
        result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "dying", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "echo", "inputSchema": schema}, {"name": "die", "inputSchema": schema}]}
    elif method == "tools/call":
        if request["params"]["name"] == "die": sys.exit(3)
        result = {"content": [{"type": "text", "text": "echo"}], "isError": False}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;

const WAIT: Duration = Duration::from_secs(20);

fn dying_config(state: &Path) -> McpServerConfig {
    let mut config = stdio_config();
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec![
        "-u".into(),
        "-c".into(),
        DYING_SERVER.into(),
        state.to_string_lossy().into_owned(),
    ];
    stdio.public_environment.clear();
    stdio.credential_environment.clear();
    config
}

fn policy(delay: Duration) -> RestartPolicy {
    RestartPolicy {
        delays: vec![delay; 3],
        window: Duration::from_secs(600),
    }
}

fn spawns(state: &Path) -> u32 {
    std::fs::read_to_string(state.join("spawns"))
        .map(|count| count.trim().parse().unwrap())
        .unwrap_or(0)
}

fn context() -> McpCallContext {
    McpCallContext {
        session_id: "session".into(),
        turn_id: 1,
        call_id: "call".into(),
        tool_generation: "generation".into(),
    }
}

fn lease(manager: &McpManager, remote_name: &str) -> McpToolLease {
    manager
        .tool_leases()
        .into_iter()
        .find(|lease| lease.binding.remote_name == remote_name)
        .unwrap()
}

/// Calls `die`; the process exits before it answers.
async fn kill(manager: &McpManager) {
    let result = lease(manager, "die")
        .call(Map::new(), context(), CancellationToken::default())
        .await;
    assert!(result.is_err(), "{result:?}");
}

/// The server's status once `done` holds for it, within [`WAIT`].
async fn wait_for(
    manager: &McpManager,
    done: impl Fn(&McpServerStatus) -> bool,
) -> McpServerStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = manager.statuses().into_iter().find(&done) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "the MCP status did not arrive: {:?}",
            manager.statuses()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn codes(manager: &McpManager, id: &McpServerId) -> Vec<String> {
    manager
        .diagnostics(id)
        .entries
        .into_iter()
        .filter(|entry| entry.stage == McpDiagnosticStage::ConnectionLost)
        .map(|entry| format!("{}: {}", entry.code, entry.message))
        .collect()
}

async fn stop(manager: McpManager, executor: &ExecutorManager, workers: BackgroundWorkerGroup) {
    manager
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(executor_registry_counts(executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[tokio::test]
async fn a_server_that_exits_is_failed_and_reconnected_and_its_old_lease_is_stale() {
    let state = tempdir().unwrap();
    let config = dying_config(state.path());
    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let manager = McpManager::new(executor.clone(), workers.clone())
        .with_reconnect_policy(policy(Duration::from_millis(10)));
    manager
        .connect_or_refresh(
            config.clone(),
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    let old_step = manager.step_binding();
    let old_echo = lease(&manager, "echo");
    assert!(!old_echo.is_stale());

    kill(&manager).await;
    let status = wait_for(&manager, |status| {
        status.phase == McpConnectionPhase::Ready && spawns(state.path()) == 2
    })
    .await;
    assert_eq!(status.reconnect, None);
    assert_eq!(status.tool_count, 2);
    assert_eq!(
        codes(&manager, &config.id),
        [
            "connection_closed: the MCP server process exited with code 3",
            "reconnect_scheduled: automatic reconnect 1 of 3 in 10 ms",
        ]
    );

    // The step bound before the server died still holds its connection,
    // whose calls say the catalog changed; the dead process stays reserved
    // until the step lets it go.
    assert!(old_echo.is_stale());
    assert_eq!(
        old_echo
            .call(Map::new(), context(), CancellationToken::default())
            .await,
        Err(McpError::CatalogChanged)
    );
    assert_eq!(executor_registry_counts(&executor), (2, 0, 2));
    drop(old_echo);
    drop(old_step);
    manager.cleanup_retired().await.unwrap();
    assert_eq!(executor_registry_counts(&executor), (1, 0, 1));
    assert!(!lease(&manager, "echo").is_stale());
    let output = lease(&manager, "echo")
        .call(Map::new(), context(), CancellationToken::default())
        .await
        .unwrap();
    assert!(!output.is_error);
    stop(manager, &executor, workers).await;
}

#[tokio::test]
async fn reconnects_stop_when_the_window_is_used_up_until_a_connect_from_outside() {
    let state = tempdir().unwrap();
    let config = dying_config(state.path());
    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let manager = McpManager::new(executor.clone(), workers.clone())
        .with_reconnect_policy(policy(Duration::from_millis(10)));
    manager
        .connect_or_refresh(
            config.clone(),
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    std::fs::write(state.path().join("broken"), "").unwrap();

    kill(&manager).await;
    let exhausted = Some(McpReconnectStatus {
        next_attempt: None,
        maximum: 3,
    });
    let status = wait_for(&manager, |status| status.reconnect == exhausted).await;
    assert_eq!(status.phase, McpConnectionPhase::Failed);
    assert!(status.last_error.is_some());
    assert_eq!(spawns(state.path()), 4);
    assert!(manager.tool_leases().is_empty());
    manager.cleanup_retired().await.unwrap();
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    let codes = codes(&manager, &config.id);
    assert_eq!(codes.len(), 5, "{codes:?}");
    assert!(codes[4].starts_with("reconnects_exhausted"), "{codes:?}");

    // The user's connect starts the server afresh, with an empty window.
    std::fs::remove_file(state.path().join("broken")).unwrap();
    let status = manager
        .connect_or_refresh(
            config.clone(),
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(status.reconnect, None);
    kill(&manager).await;
    wait_for(&manager, |status| {
        status.phase == McpConnectionPhase::Ready && spawns(state.path()) == 6
    })
    .await;
    stop(manager, &executor, workers).await;
}

#[tokio::test]
async fn shutdown_cancels_a_scheduled_reconnect() {
    let state = tempdir().unwrap();
    let config = dying_config(state.path());
    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let manager = McpManager::new(executor.clone(), workers.clone())
        .with_reconnect_policy(policy(Duration::from_secs(600)));
    manager
        .connect_or_refresh(
            config,
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
        .await
        .unwrap();

    kill(&manager).await;
    let scheduled = Some(McpReconnectStatus {
        next_attempt: Some(1),
        maximum: 3,
    });
    let status = wait_for(&manager, |status| status.reconnect == scheduled).await;
    assert_eq!(status.phase, McpConnectionPhase::Failed);
    assert_eq!(
        status.last_error.as_deref(),
        Some("the MCP server process exited with code 3")
    );
    let started = Instant::now();
    stop(manager, &executor, workers).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(spawns(state.path()), 1);
}
