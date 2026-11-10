//! What a guest may reach of the Agent: it views its workspace (as granted)
//! and its settings, which are also its configuration; every other service
//! is out of reach of `view` and `invoke`. A guest provides only its
//! `plugin`: one that provides another service is stopped.

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::WorkspaceAccess;
use hanafubuki_agent_plugin_packages::GuestState;
use serde_json::{json, Map, Value};

use crate::fixture::{eventually, running, Fixture, Options};

fn settings(board: &str) -> Map<String, Value> {
    Map::from_iter([("board".to_string(), json!(board))])
}

pub async fn a_guest_views_only_its_workspace_and_its_settings() {
    let fixture = Fixture::start(Options::default()).await;
    let mut package = fixture.package("demo", "run");
    let id = package.id.key();
    package.permissions.workspace = WorkspaceAccess::Read;
    package.granted = Some(package.permissions.clone());
    package.settings = settings("B-1");
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;

    let guest = package.clone();
    let view = |key: &str| fixture.call(&guest, "view", json!(key));
    let workspace = ResolvedPath::resolve(&fixture.workspace).unwrap();
    assert_eq!(
        view("workspace").await,
        Ok(json!({ "access": "read", "roots": [workspace.native_text()] }))
    );
    assert_eq!(view("settings").await, Ok(json!({ "board": "B-1" })));
    assert_eq!(
        fixture.call(&package, "config", Value::Null).await,
        Ok(json!({ "board": "B-1" }))
    );
    // The guest host's own services, the registries, and what no fiber of
    // the guest's declares are all refused.
    for key in [
        "loader",
        "plugin-faults",
        "agent-tool-sources",
        "agent-context",
        "agent-tools",
        "anything",
    ] {
        let refused = view(key).await;
        assert!(refused.is_err(), "{key}: {refused:?}");
    }
    for key in ["plugin", "settings", "workspace", "loader", "anything"] {
        let refused = fixture
            .call(&package, "invoke", json!({ "key": key, "method": "echo" }))
            .await;
        assert!(refused.is_err(), "{key}: {refused:?}");
    }

    // New settings restart the guest with them.
    package.settings = settings("B-2");
    fixture.apply(vec![package.clone()]).await;
    fixture
        .wait(&id, "restart with its settings", |status| {
            running(status) && status.process_id != first.process_id
        })
        .await;
    assert_eq!(view("settings").await, Ok(json!({ "board": "B-2" })));
    assert_eq!(
        fixture.call(&package, "config", Value::Null).await,
        Ok(json!({ "board": "B-2" }))
    );

    // Without workspace access, the workspace has no roots.
    package.permissions.workspace = WorkspaceAccess::None;
    package.granted = Some(package.permissions.clone());
    fixture.apply(vec![package.clone()]).await;
    // A new grant is a new start: wait for its view.
    let mut answer = Err(String::new());
    let deadline = std::time::Instant::now() + crate::fixture::DEADLINE;
    while answer != Ok(json!({ "access": "none", "roots": [] })) {
        assert!(std::time::Instant::now() < deadline, "{answer:?}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        answer = view("workspace").await;
    }
}

pub async fn a_guest_that_provides_another_service_is_stopped() {
    let fixture = Fixture::start(Options::default()).await;

    // While it starts: it restarts until the restart policy gives up.
    let package = fixture.package("thief", "provide-apply");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let failed = fixture
        .wait(&id, "fail", |status| status.state == GuestState::Failed)
        .await;
    assert!(
        failed
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("stolen"),
        "{failed:?}"
    );
    assert!(fixture.plugin.context().bindings("stolen").is_empty());

    // While it runs: it fails at once, and restarts as after any fault.
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;
    let _ = fixture
        .call(&package, "provide", json!("agent-tools"))
        .await;
    let second = fixture
        .wait(&id, "restart", |status| {
            running(status) && status.process_id != first.process_id
        })
        .await;
    assert_eq!(second.restarts, 1);
    eventually("the binding withdrawn", || {
        fixture.plugin.context().bindings("agent-tools").is_empty()
    })
    .await;
}
