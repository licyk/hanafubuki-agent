//! A code package (this project's format, with a runtime): its guest waits
//! for the user's grant of its permissions, then starts with the `plugins`
//! row under the platform's sandbox. This test binary never registered the
//! sandbox helper, so on Linux and Windows the guest does not start at all
//! rather than start unconfined. The guest host itself, with real guests,
//! is `hanafubuki-agent-plugin-packages`'s `guests` suite.

use crate::{AgentPackageSource, GuestState, PackagePermissions, WorkspaceAccess};

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
