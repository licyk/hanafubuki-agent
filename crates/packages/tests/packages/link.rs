//! A package linked for development: recorded by reference, listed as the
//! directory as it is now, never copied, pinned or written into; removing
//! it removes its record and keeps the directory; it and an install of the
//! same id do not replace each other.

use std::fs;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::{PackagePermissions, PackageSource, PluginId, WorkspaceAccess};

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
        serde_json::from_str(&fs::read_to_string(fixture.plugins().join("install.json")).unwrap())
            .unwrap();
    let entry = &record["packages"]["sample@local"];
    assert_eq!(entry["source"]["type"], "linked");
    assert_eq!(entry["source"]["path"], native.to_str().unwrap());
    assert!(entry["contentHash"].is_null());

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
    let record = fs::read_to_string(fixture.plugins().join("install.json")).unwrap();
    assert!(!record.contains("sample@local"), "{record}");
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
