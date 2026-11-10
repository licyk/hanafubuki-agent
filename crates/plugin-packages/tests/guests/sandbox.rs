//! The plugin profile on the platform's sandbox: a guest started from its
//! package (`./bin/guest`) reads its package and its data, writes only its
//! data, and reaches the workspace only as granted. Checked on Linux
//! (Landlock); a machine without the sandbox skips the check.

use std::fs;

use hanafubuki_agent_packages::{content_hash, PackageRuntime, ProcessRuntime, WorkspaceAccess};
use hanafubuki_agent_plugin_packages::GuestPackage;
use hanafubuki_agent_sandbox::{Sandbox, SandboxStatus};
use serde_json::json;

use crate::fixture::{running, Fixture, Options};
use crate::guest::GUEST_ARG;

pub async fn the_sandbox_keeps_a_guest_to_its_package_its_data_and_its_grant() {
    // macOS's Seatbelt restricts reads too, but this check has not run
    // there yet; Windows's restricted token does not restrict them.
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: checked on Linux only");
        return;
    }
    let sandbox = Sandbox::detect();
    if let SandboxStatus::Unavailable { reason } = sandbox.status() {
        eprintln!("skipped: the sandbox is unavailable here ({reason})");
        return;
    }
    let fixture = Fixture::start(Options {
        sandbox: Some(sandbox),
        ..Options::default()
    })
    .await;
    let mut package = installed_binary(fixture.package("demo", "run"));
    let id = package.id.key();
    let outside = fixture.directory.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret"), "secret").unwrap();
    fs::write(fixture.workspace.join("notes"), "workspace").unwrap();
    let path = |path: &std::path::Path| json!(path.to_str().unwrap());

    fixture.apply(vec![package.clone()]).await;
    fixture.wait(&id, "start", running).await;
    assert_eq!(
        fixture
            .call(&package, "read", path(&package.root.join("package.txt")))
            .await,
        Ok(json!("inside the package"))
    );
    assert!(fixture
        .call(&package, "read", path(&outside.join("secret")))
        .await
        .is_err());
    assert!(fixture
        .call(&package, "read", path(&fixture.workspace.join("notes")))
        .await
        .is_err());
    let data_file = package.data_root.join("written");
    assert!(fixture
        .call(&package, "write", path(&data_file))
        .await
        .is_ok());
    assert!(fixture
        .call(&package, "write", path(&package.root.join("written")))
        .await
        .is_err());
    assert!(!package.root.join("written").exists());

    // Read access to the workspace, once asked for and granted, is a new
    // start under a new profile.
    package.permissions.workspace = WorkspaceAccess::Read;
    package.granted = Some(package.permissions.clone());
    fixture.apply(vec![package.clone()]).await;
    fixture.wait(&id, "start with the workspace", running).await;
    assert_eq!(
        fixture
            .call(&package, "read", path(&fixture.workspace.join("notes")))
            .await,
        Ok(json!("workspace"))
    );
    assert!(fixture
        .call(&package, "write", path(&fixture.workspace.join("written")))
        .await
        .is_err());
}

/// `package` with this binary inside it, started as `./bin/guest`.
fn installed_binary(mut package: GuestPackage) -> GuestPackage {
    let bin = package.root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let guest = bin.join(if cfg!(windows) { "guest.exe" } else { "guest" });
    if fs::hard_link(std::env::current_exe().unwrap(), &guest).is_err() {
        fs::copy(std::env::current_exe().unwrap(), &guest).unwrap();
    }
    package.runtime = PackageRuntime::Process(ProcessRuntime {
        command: vec![
            "./bin/guest".to_string(),
            GUEST_ARG.to_string(),
            package.name.clone(),
            "run".to_string(),
        ],
        interpreter: false,
        protocol: None,
    });
    package.content_hash = Some(content_hash(&package.root).unwrap());
    package
}
