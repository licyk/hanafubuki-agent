//! A code package (this project's format, with a runtime): its guest waits
//! for the user's grant of its permissions, then starts with the `plugins`
//! row under the platform's sandbox. This test binary never registered the
//! sandbox helper, so on Linux and Windows the guest does not start at all
//! rather than start unconfined. A WASM guest needs neither grant nor
//! sandbox. The guest host itself, with real guests, is
//! `hanafubuki-agent-plugin-packages`'s `guests` suite.

use crate::{AgentPackageSource, GuestState, PackagePermissions, RuntimeKind, WorkspaceAccess};

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_code_package_waits_for_its_grant_and_never_starts_unconfined() {
    let (service, paths, _) = service_with_config("packages-code", |_, _| {});
    let source = paths.app_data_dir.join("sources/coder");
    write(
        &source.join(".hanafubuki-plugin/plugin.json"),
        &json!({
            "name": "coder",
            "version": "1.0.0",
            "runtime": { "type": "process", "command": ["./bin/guest"] },
            "permissions": { "workspace": "none", "env": ["CODER_TOKEN"] },
        })
        .to_string(),
    );
    write(&source.join("bin/guest"), "not a program");
    service.start_plugins().await;
    let install = service
        .install_package(AgentPackageSource::Directory {
            path: source,
            marketplace: None,
        })
        .await
        .unwrap();

    let requested = PackagePermissions {
        network: false,
        workspace: WorkspaceAccess::None,
        env: vec!["CODER_TOKEN".to_string()],
    };
    // The install asks for the consent its guest waits for.
    assert_eq!(install.permissions, Some(requested.clone()));
    let guest = package(&service, "coder@local").await.guest.unwrap();
    assert_eq!(guest.permissions, requested);
    assert_eq!(guest.granted_permissions, None);
    assert_eq!(guest.status.state, GuestState::Blocked);

    service
        .grant_package_permissions("coder@local", requested.clone())
        .await
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    let guest = loop {
        let guest = package(&service, "coder@local").await.guest.unwrap();
        if guest.status.state == GuestState::Failed {
            break guest;
        }
        assert!(Instant::now() < deadline, "{guest:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(guest.granted_permissions, Some(requested));
    assert_eq!(guest.status.process_id, None);
    if cfg!(not(target_os = "macos")) {
        let error = guest.status.error.unwrap();
        assert!(error.contains("sandbox is unavailable"), "{error}");
    }
    // A start that failed is retried by hand only.
    let retried = service.retry_package("coder@local").await.unwrap();
    assert!(retried.guest.is_some());
    assert!(service.retry_package("nothing@local").await.is_err());
    finish(service, paths).await;
}

/// A module declaring the plugin `wasmer`, which provides `plugin` and
/// answers each call with its request.
const WASM_GUEST: &str = r#"(module
  (import "cordis" "read" (func $read (param i32)))
  (import "cordis" "write" (func $write (param i32 i32)))
  (import "cordis" "provide" (func $provide (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{\"plugin\":{\"name\":\"wasmer\",\"provide\":[\"plugin\"]}}")
  (data (i32.const 64) "plugin{}")
  (func (export "cordis_meta") (result i32) (call $write (i32.const 0) (i32.const 49)) (i32.const 0))
  (func (export "cordis_apply") (param i32) (result i32)
    (drop (call $provide (i32.const 64) (i32.const 6) (i32.const 70) (i32.const 2)))
    (i32.const 0))
  (func (export "cordis_call") (param $len i32) (result i32)
    (call $read (i32.const 4096))
    (call $write (i32.const 4096) (local.get $len))
    (i32.const 0)))"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_package_asks_for_nothing_and_runs_without_the_sandbox() {
    let (service, paths, _) = service_with_config("packages-wasm", |_, _| {});
    let source = paths.app_data_dir.join("sources/wasmer");
    write(
        &source.join(".hanafubuki-plugin/plugin.json"),
        &json!({
            "name": "wasmer",
            "version": "1.0.0",
            "runtime": { "type": "wasm", "module": "./guest.wasm" },
        })
        .to_string(),
    );
    std::fs::write(
        source.join("guest.wasm"),
        wat::parse_str(WASM_GUEST).unwrap(),
    )
    .unwrap();
    service.start_plugins().await;
    let install = service
        .install_package(AgentPackageSource::Directory {
            path: source,
            marketplace: None,
        })
        .await
        .unwrap();

    assert_eq!(install.permissions, None);
    let deadline = Instant::now() + Duration::from_secs(60);
    let guest = loop {
        let guest = package(&service, "wasmer@local").await.guest.unwrap();
        if guest.status.state == GuestState::Running {
            break guest;
        }
        assert_ne!(guest.status.state, GuestState::Failed, "{guest:?}");
        assert!(Instant::now() < deadline, "{guest:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(guest.runtime, RuntimeKind::Wasm);
    assert_eq!(guest.ungranted(), None);
    assert_eq!(guest.granted_permissions, None);
    finish(service, paths).await;
}
