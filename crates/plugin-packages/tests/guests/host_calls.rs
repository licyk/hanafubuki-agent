//! What a guest may reach of the Agent: it views its workspace (as granted)
//! and its settings, which are also its configuration; every other service
//! is out of reach of `view` and `invoke`. A guest provides only its
//! `plugin`: kardia-hmr refuses it any other, which is the guest's error.

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::WorkspaceAccess;
use hanafubuki_agent_plugin_packages::GuestState;
use serde_json::{json, Map, Value};

use crate::fixture::{running, Fixture, Options};

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

pub async fn a_guest_that_provides_another_service_is_refused() {
    let fixture = Fixture::start(Options::default()).await;

    // While it starts: an `apply` that returns the refusal fails the start.
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
            .contains("cannot provide \"stolen\""),
        "{failed:?}"
    );
    assert_eq!(failed.restarts, 0);
    assert!(fixture.plugin.context().bindings("stolen").is_empty());

    // While it runs: the refusal is the call's error; nothing was provided,
    // so the guest keeps running.
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;
    let refused = fixture
        .call(&package, "provide", json!("agent-tools"))
        .await
        .unwrap_err();
    assert!(
        refused.contains("cannot provide \"agent-tools\""),
        "{refused}"
    );
    assert!(fixture.plugin.context().bindings("agent-tools").is_empty());
    let status = fixture.host.status(&id).unwrap();
    assert!(
        running(&status) && status.restarts == 0 && status.process_id == first.process_id,
        "{status:?}"
    );
    assert_eq!(fixture.call(&package, "echo", json!(1)).await, Ok(json!(1)));
}

pub async fn a_guest_calls_the_agent_it_is_granted() {
    let fixture = Fixture::start(Options::default()).await;
    std::fs::write(fixture.workspace.join("notes.txt"), "first\nsecond\n").unwrap();
    std::fs::create_dir(fixture.workspace.join("src")).unwrap();
    std::fs::write(fixture.workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(fixture.workspace.join("protected/secret.txt"), "hidden\n").unwrap();
    let mut package = fixture.package("demo", "run");
    let id = package.id.key();
    package.settings = settings("B-1");
    fixture.apply(vec![package.clone()]).await;
    fixture.wait(&id, "start", running).await;

    let agent =
        |method: &str, args: Value| json!({ "key": "agent", "method": method, "args": args });
    // The settings are the ones it was configured with.
    assert_eq!(
        fixture
            .call(&package, "invoke", agent("settings.get", Value::Null))
            .await,
        Ok(json!({ "board": "B-1" }))
    );
    // Without the grant, the workspace is not readable, and neither method
    // reaches it.
    for (method, args) in [
        ("workspace.list", json!({ "path": "." })),
        ("workspace.read", json!({ "path": "notes.txt" })),
        (
            "workspace.search",
            json!({ "path": ".", "query": "second" }),
        ),
    ] {
        let refused = fixture
            .call(&package, "invoke", agent(method, args))
            .await
            .unwrap_err();
        assert!(refused.contains("not readable"), "{method}: {refused}");
    }

    // Granted, both read what the workspace policy allows.
    package.permissions.workspace = WorkspaceAccess::Read;
    package.granted = Some(package.permissions.clone());
    fixture.apply(vec![package.clone()]).await;
    // A new grant is a new start: wait for its listing.
    let mut listed = Err(String::new());
    let deadline = std::time::Instant::now() + crate::fixture::DEADLINE;
    while !listed.as_ref().is_ok_and(has_entries) {
        assert!(std::time::Instant::now() < deadline, "{listed:?}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        listed = fixture
            .call(
                &package,
                "invoke",
                agent("workspace.list", json!({ "path": "src", "depth": 2 })),
            )
            .await;
    }
    assert_eq!(
        fixture
            .call(
                &package,
                "invoke",
                agent("workspace.read", json!({ "path": "notes.txt" })),
            )
            .await
            .map(|read| read["content"].clone()),
        Ok(json!("first\nsecond\n"))
    );
    // A protected subtree is the policy's to refuse.
    let refused = fixture
        .call(
            &package,
            "invoke",
            agent("workspace.read", json!({ "path": "protected/secret.txt" })),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("protected"), "{refused}");
    // A method no one registered says so.
    let refused = fixture
        .call(&package, "invoke", agent("nothing.at.all", json!({})))
        .await
        .unwrap_err();
    assert!(refused.contains("no method nothing.at.all"), "{refused}");

    // The search reads what the policy allows, and says where it matched.
    let mut found = Err(String::new());
    while !found.as_ref().is_ok_and(|results: &Value| {
        results["matches"].as_array().is_some_and(|matches| {
            matches.iter().any(|entry| {
                entry["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("main.rs"))
            })
        })
    }) {
        assert!(
            std::time::Instant::now() < deadline,
            "the search found nothing: {found:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        found = fixture
            .call(
                &package,
                "invoke",
                agent(
                    "workspace.search",
                    json!({ "path": "src", "query": "main" }),
                ),
            )
            .await;
    }
    let refused = fixture
        .call(
            &package,
            "invoke",
            agent(
                "workspace.search",
                json!({ "path": "protected", "query": "hidden" }),
            ),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("protected"), "{refused}");
}

fn has_entries(value: &Value) -> bool {
    value["entries"]
        .as_array()
        .is_some_and(|entries| !entries.is_empty())
}
