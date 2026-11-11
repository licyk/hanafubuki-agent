//! A package linked for development: recorded by reference, listed as the
//! directory as it is now, never copied, pinned or written into; removing
//! it removes its record and keeps the directory; it and an install of the
//! same id do not replace each other; its record, in `linked.json`, outlives
//! an older build's rewrite of `install.json`.

use std::fs;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::{
    PackagePermissions, PackageSource, PackageStore, PluginId, WorkspaceAccess,
};

use super::{fixture, write, write_plugin};

fn id(plugin: &str) -> PluginId {
    PluginId::new(plugin, "local").unwrap()
}

#[test]
fn a_linked_package_is_its_directory_recorded_by_reference() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "work", "sample", Some("0.1.0"));
    write(
        &source.join("commands/hello.md"),
        "---\ndescription: Hi.\n---\nHi.\n",
    );
    let sample = id("sample");

    let outcome = fixture.store.link_directory(&source, &sample).unwrap();

    let native = ResolvedPath::resolve(&source)
        .unwrap()
        .native()
        .to_path_buf();
    assert_eq!(outcome.root, native);
    assert!(
        outcome
            .issues
            .iter()
            .any(|issue| issue.contains("commands")),
        "{:?}",
        outcome.issues
    );
    assert!(!fixture.plugins().join("cache").exists());
    // Nothing is written into the directory: its commands stay commands.
    assert!(!source
        .join(".codex-plugin/migrated-command-skills")
        .exists());
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.plugins().join("linked.json")).unwrap())
            .unwrap();
    let entry = &record["packages"]["sample@local"];
    assert_eq!(entry["path"], native.to_str().unwrap());
    assert_eq!(entry["version"], "0.1.0");
    // `install.json` holds the cache's packages alone.
    let installed = fs::read_to_string(fixture.plugins().join("install.json")).unwrap();
    assert!(!installed.contains("sample@local"), "{installed}");

    let installed = fixture.store.installed();
    let [package] = installed.as_slice() else {
        panic!("one package: {installed:?}");
    };
    assert_eq!(package.id, sample);
    assert_eq!(package.root, native);
    assert_eq!(package.linked(), Some(native.as_path()));
    assert_eq!(package.version, "0.1.0");
    assert!(package.issues.is_empty(), "{:?}", package.issues);

    // Read as the directory is now.
    write(
        &source.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample","version":"0.2.0"}"#,
    );
    assert_eq!(fixture.store.read(&sample).unwrap().version, "0.2.0");
    fs::remove_dir_all(&source).unwrap();
    let gone = fixture.store.read(&sample).unwrap();
    assert!(gone.manifest.is_none());
    assert!(
        gone.issues[0].contains("its linked directory"),
        "{:?}",
        gone.issues
    );
}

#[test]
fn removing_a_linked_package_keeps_its_directory() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "work", "sample", None);
    let sample = id("sample");
    fixture.store.link_directory(&source, &sample).unwrap();
    let granted = PackagePermissions {
        network: false,
        workspace: WorkspaceAccess::Read,
        env: Vec::new(),
    };
    fixture
        .store
        .grant_permissions(&sample, granted.clone())
        .unwrap();
    // Linked again: the grant is kept, as an install keeps it.
    fixture.store.link_directory(&source, &sample).unwrap();
    let record = fixture.store.read(&sample).unwrap().record.unwrap();
    assert_eq!(record.granted_permissions, Some(granted));
    assert!(matches!(record.source, Some(PackageSource::Linked { .. })));

    let outcome = fixture.store.uninstall(&sample, true).unwrap();

    assert!(outcome.removed);
    assert!(source.join(".codex-plugin/plugin.json").is_file());
    assert!(source.join("skills/SKILL.md").is_file());
    assert!(fixture.store.installed().is_empty());
    assert!(fixture.store.read(&sample).is_err());
    let record = fs::read_to_string(fixture.plugins().join("linked.json")).unwrap();
    assert!(!record.contains("sample@local"), "{record}");
}

/// A build that predates links rewrites `install.json` alone, keeping an
/// entry it reads with an unknown source as `"source": null`, or dropping
/// one it never saw: the link, in `linked.json`, stays.
#[test]
fn a_link_survives_a_rewrite_of_install_json_by_an_older_build() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "work", "sample", None);
    let other = write_plugin(fixture.path(), "other", "other", None);
    let sample = id("sample");
    fixture.store.link_directory(&source, &sample).unwrap();
    fixture
        .store
        .install_directory(&other, &id("other"))
        .unwrap();
    let granted = PackagePermissions {
        network: true,
        workspace: WorkspaceAccess::None,
        env: Vec::new(),
    };
    fixture
        .store
        .grant_permissions(&sample, granted.clone())
        .unwrap();

    for rewrite in [
        r#"{"packages":{"sample@local":{"version":null,"format":"codex","source":null,"contentHash":null,"grantedPermissions":null}}}"#,
        r#"{"packages":{}}"#,
    ] {
        fs::write(fixture.plugins().join("install.json"), rewrite).unwrap();

        let package = fixture.store.read(&sample).unwrap();
        assert_eq!(package.linked(), Some(package.root.as_path()));
        let record = package.record.unwrap();
        assert_eq!(record.granted_permissions, Some(granted.clone()));
        let linked: Vec<_> = fixture
            .store
            .installed()
            .into_iter()
            .filter(|package| package.linked().is_some())
            .map(|package| package.id)
            .collect();
        assert_eq!(linked, std::slice::from_ref(&sample));
        assert!(fixture
            .store
            .install_directory(&source, &sample)
            .unwrap_err()
            .to_string()
            .contains("is linked"));
    }

    // A later grant goes to the link, not to the stale `install.json` entry.
    fs::write(
        fixture.plugins().join("install.json"),
        r#"{"packages":{"sample@local":{"source":null}}}"#,
    )
    .unwrap();
    fixture
        .store
        .grant_permissions(&sample, PackagePermissions::default())
        .unwrap();
    let record = fixture.store.read(&sample).unwrap().record.unwrap();
    assert_eq!(
        record.granted_permissions,
        Some(PackagePermissions::default())
    );
    fixture.store.uninstall(&sample, false).unwrap();
    assert!(fixture.store.read(&sample).is_err());
    let installed = fs::read_to_string(fixture.plugins().join("install.json")).unwrap();
    assert!(!installed.contains("sample@local"), "{installed}");
}

/// An `install.json` entry of source `linked`, which the first build of
/// links wrote, is read as a link; the store's next write moves it into
/// `linked.json`.
#[test]
fn an_earlier_builds_link_in_install_json_moves_to_linked_json() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "work", "sample", Some("0.1.0"));
    let other = write_plugin(fixture.path(), "other", "other", None);
    let native = ResolvedPath::resolve(&source)
        .unwrap()
        .native()
        .to_path_buf();
    let entry = serde_json::json!({"packages": {"sample@local": {
        "version": "0.1.0",
        "format": "codex",
        "source": {"type": "linked", "path": native},
        "contentHash": null,
        "grantedPermissions": {"network": true},
    }}});
    fs::create_dir_all(fixture.plugins()).unwrap();
    fs::write(fixture.plugins().join("install.json"), entry.to_string()).unwrap();

    let package = fixture.store.read(&id("sample")).unwrap();
    assert_eq!(package.linked(), Some(native.as_path()));
    assert!(!fixture.plugins().join("linked.json").exists());

    fixture
        .store
        .install_directory(&other, &id("other"))
        .unwrap();
    let linked = fs::read_to_string(fixture.plugins().join("linked.json")).unwrap();
    assert!(linked.contains("sample@local"), "{linked}");
    let installed = fs::read_to_string(fixture.plugins().join("install.json")).unwrap();
    assert!(!installed.contains("sample@local"), "{installed}");
    let record = fixture.store.read(&id("sample")).unwrap().record.unwrap();
    assert!(record.granted_permissions.unwrap().network);
}

/// `read_installed` lists what the store lists, and changes nothing: the
/// staging area stays, and a record file that does not read is not moved
/// aside.
#[test]
fn reading_the_installed_packages_without_a_store_writes_nothing() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "work", "sample", None);
    let other = write_plugin(fixture.path(), "other", "other", None);
    fixture
        .store
        .link_directory(&source, &id("sample"))
        .unwrap();
    fixture
        .store
        .install_directory(&other, &id("other"))
        .unwrap();
    let staged = fixture.plugins().join(".staging/marker");
    write(&staged, "work in progress");

    let listed = PackageStore::read_installed(&fixture.plugins());
    assert_eq!(listed, fixture.store.installed());
    assert_eq!(listed.len(), 2);
    assert!(staged.is_file());

    fs::write(fixture.plugins().join("linked.json"), "{not json").unwrap();
    let listed = PackageStore::read_installed(&fixture.plugins());
    assert_eq!(listed.len(), 1);
    assert_eq!(
        fs::read_to_string(fixture.plugins().join("linked.json")).unwrap(),
        "{not json"
    );
    assert!(!fixture.plugins().join(".recovery").exists());
    assert!(staged.is_file());
}

#[test]
fn a_link_and_an_install_of_one_id_do_not_replace_each_other() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "work", "sample", None);
    let other = write_plugin(fixture.path(), "other", "other", None);
    let sample = id("sample");

    fixture.store.link_directory(&source, &sample).unwrap();
    let refused = fixture
        .store
        .install_directory(&source, &sample)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("is linked"), "{refused}");
    assert!(!fixture.plugins().join("cache").exists());

    fixture
        .store
        .install_directory(&other, &id("other"))
        .unwrap();
    let refused = fixture
        .store
        .link_directory(&other, &id("other"))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("is installed"), "{refused}");
    let wrong = fixture
        .store
        .link_directory(&other, &id("sample"))
        .unwrap_err()
        .to_string();
    assert!(wrong.contains("does not match"), "{wrong}");
    let missing = fixture
        .store
        .link_directory(&fixture.path().join("absent"), &id("absent"))
        .unwrap_err()
        .to_string();
    assert!(missing.contains("cannot be used"), "{missing}");

    let installed = fixture.store.installed();
    assert_eq!(installed.len(), 2, "{installed:?}");
    assert!(installed[0].linked().is_none());
    assert!(installed[1].linked().is_some());
}
