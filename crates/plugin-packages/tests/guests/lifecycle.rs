//! A guest starts with its package, stops when it is turned off or
//! uninstalled, waits for its permissions, gets only its own environment,
//! and its process is the instance's: in its owner group and its record,
//! reaped by the next start's sweep, stopped with the guest host.

use std::collections::BTreeMap;
use std::time::Duration;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::PackagePermissions;
use hanafubuki_agent_plugin_packages::GuestState;
use hanafubuki_agent_process::executor::manager::terminate_owner_group;
use serde_json::{json, Value};

use crate::fixture::{eventually, running, Fixture, Options};

pub async fn a_guest_starts_is_disabled_enabled_and_uninstalled() {
    let fixture = Fixture::start(Options::default()).await;
    let mut package = fixture.package("demo", "run");
    let id = package.id.key();

    fixture.apply(vec![package.clone()]).await;
    let first = fixture
        .wait(&id, "start", |status| {
            running(status) && status.stderr_tail.contains("the test guest applies")
        })
        .await;
    assert!(first.process_id.is_some());
    assert_eq!(first.error, None);
    assert_eq!(
        fixture.call(&package, "echo", json!({ "a": 1 })).await,
        Ok(json!({ "a": 1 }))
    );
    assert_eq!(fixture.processes(), 1);

    package.enabled = false;
    fixture.apply(vec![package.clone()]).await;
    fixture
        .wait(&id, "stop", |status| status.state == GuestState::Stopped)
        .await;
    fixture.wait_for_processes(0).await;
    assert!(fixture.call(&package, "echo", Value::Null).await.is_err());

    package.enabled = true;
    fixture.apply(vec![package.clone()]).await;
    let second = fixture.wait(&id, "start again", running).await;
    assert_ne!(second.process_id, first.process_id);
    assert_eq!(second.restarts, 0);

    fixture.apply(Vec::new()).await;
    assert_eq!(fixture.host.status(&id), None);
    fixture.wait_for_processes(0).await;
    // The tree mirrors the settings: applying them writes nothing back.
    assert!(fixture.settings.0.lock().is_empty());
}

pub async fn a_guest_without_its_permissions_starts_only_once_they_are_granted() {
    let fixture = Fixture::start(Options::default()).await;
    let mut package = fixture.package("demo", "run");
    let id = package.id.key();
    let requested = package.permissions.clone();

    package.granted = None;
    fixture.apply(vec![package.clone()]).await;
    let blocked = fixture.host.status(&id).unwrap();
    assert_eq!(blocked.state, GuestState::Blocked);
    assert!(blocked.error.unwrap().contains("not granted"));

    package.granted = Some(PackagePermissions {
        env: Vec::new(),
        ..requested.clone()
    });
    fixture.apply(vec![package.clone()]).await;
    let blocked = fixture.host.status(&id).unwrap();
    assert_eq!(blocked.state, GuestState::Blocked);
    assert!(
        blocked
            .error
            .as_deref()
            .unwrap()
            .contains("env HANAFUBUKI_TEST_GRANTED"),
        "{blocked:?}"
    );
    assert_eq!(fixture.processes(), 0);

    package.granted = Some(requested);
    fixture.apply(vec![package.clone()]).await;
    fixture.wait(&id, "start once granted", running).await;
}

pub async fn a_guest_gets_only_its_environment_and_runs_in_its_data_directory() {
    std::env::set_var("HANAFUBUKI_TEST_GRANTED", "granted");
    std::env::set_var("HANAFUBUKI_TEST_NOT_GRANTED", "not for the guest");
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    fixture.apply(vec![package.clone()]).await;
    fixture.wait(&package.id.key(), "start", running).await;

    let mut env: BTreeMap<String, String> =
        serde_json::from_value(fixture.call(&package, "env", Value::Null).await.unwrap()).unwrap();
    // macOS adds this to a process whose environment was cleared.
    if cfg!(target_os = "macos") {
        env.remove("__CF_USER_TEXT_ENCODING");
    }
    let root = ResolvedPath::resolve(&package.root).unwrap();
    let data = ResolvedPath::resolve(&package.data_root).unwrap();
    let mut expected = BTreeMap::from([
        ("HANAFUBUKI_TEST_GRANTED".to_string(), "granted".to_string()),
        ("PLUGIN_DATA".to_string(), data.native_text().to_string()),
        ("PLUGIN_ROOT".to_string(), root.native_text().to_string()),
    ]);
    if cfg!(windows) {
        expected.insert(
            "SystemRoot".to_string(),
            std::env::var("SystemRoot").unwrap(),
        );
    }
    assert_eq!(env, expected);
    let cwd = fixture.call(&package, "cwd", Value::Null).await.unwrap();
    let cwd = ResolvedPath::resolve(cwd.as_str().unwrap()).unwrap();
    assert_eq!(cwd, data);
}

pub async fn a_guest_process_is_the_instances_and_stops_with_the_guest_host() {
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;
    assert_eq!(fixture.processes(), 1);

    // The instance's stop sweeps its owner group, the guest's process too;
    // a guest that loses its process that way is a fault like any other.
    let left = terminate_owner_group(&fixture.executor, &fixture.group, Duration::from_secs(20));
    assert!(left.is_empty(), "{left:?}");
    let second = fixture
        .wait(&id, "restart after the sweep", |status| {
            running(status) && status.process_id != first.process_id
        })
        .await;
    assert_eq!(second.restarts, 1);

    // The process group is recorded while it runs, so the next start reaps
    // it after a crash. Only Linux records one: macOS has no leader identity
    // the ledger reads yet.
    if cfg!(target_os = "linux") {
        let pid = second.process_id.unwrap();
        let recorded =
            std::fs::read_to_string(fixture.directory.path().join("process-groups.json")).unwrap();
        assert!(recorded.contains(&format!("\"pgid\":{pid}")), "{recorded}");
        let reap = fixture.record.reap_leftovers();
        assert_eq!(reap.terminated, vec![pid]);
        fixture
            .wait(&id, "restart after the reap", |status| {
                running(status) && status.process_id != second.process_id
            })
            .await;
    }

    fixture.plugin.dispose().await.unwrap();
    fixture
        .wait(&id, "stop with the guest host", |status| {
            status.state == GuestState::Stopped
        })
        .await;
    fixture.wait_for_processes(0).await;
    // The session forgets its group just after it announces its end.
    let record = fixture.directory.path().join("process-groups.json");
    eventually("an empty process group record", || {
        !std::fs::read_to_string(&record)
            .unwrap_or_default()
            .contains("\"pgid\"")
    })
    .await;
}
