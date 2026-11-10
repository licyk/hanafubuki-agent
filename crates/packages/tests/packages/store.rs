// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/store_tests.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the remote install metadata tests are not
// ported; fallback manifests are installed from a marketplace; the install
// record, the content hash, uninstall's purge and the staging sweep are
// added.

use std::fs;

use hanafubuki_agent_packages::{
    content_hash, load_marketplace, migrated_command_skills_root, ManifestFormat, PackageSource,
    PackageStore, PluginId,
};
use hanafubuki_agent_process::executor::ExecutorManager;

use super::{fixture, write, write_plugin};

fn id(plugin: &str, marketplace: &str) -> PluginId {
    PluginId::new(plugin, marketplace).unwrap()
}

#[test]
fn install_copies_plugin_into_its_marketplace_cache() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "sample-plugin", "sample-plugin", None);

    let outcome = fixture
        .store
        .install_directory(&source, &id("sample-plugin", "debug"))
        .unwrap();

    let installed = fixture.plugins().join("cache/debug/sample-plugin/local");
    assert_eq!(outcome.version, "local");
    assert_eq!(outcome.root, installed);
    assert!(installed.join(".codex-plugin/plugin.json").is_file());
    assert!(installed.join("skills/SKILL.md").is_file());
}

#[test]
fn install_accepts_manifest_mcp_server_objects() {
    let fixture = fixture();
    let source = fixture.path().join("counter-sample");
    write(
        &source.join(".codex-plugin/plugin.json"),
        r#"{"name": "counter-sample", "version": "1.1.1",
            "mcpServers": {"counter": {"type": "http", "url": "https://sample.example/counter/mcp"}}}"#,
    );

    let outcome = fixture
        .store
        .install_directory(&source, &id("counter-sample", "debug"))
        .unwrap();

    assert_eq!(outcome.version, "1.1.1");
    assert_eq!(
        outcome.root,
        fixture.plugins().join("cache/debug/counter-sample/1.1.1")
    );
}

#[test]
fn install_uses_manifest_name_for_destination_and_key() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "source-dir", "manifest-name", None);

    let outcome = fixture
        .store
        .install_directory(&source, &id("manifest-name", "market"))
        .unwrap();

    assert_eq!(
        outcome.root,
        fixture.plugins().join("cache/market/manifest-name/local")
    );
}

#[test]
fn plugin_root_and_data_root_derive_from_the_key() {
    let fixture = fixture();
    let sample = id("sample", "debug");

    assert_eq!(
        fixture.store.package_root(&sample, "local"),
        fixture.plugins().join("cache/debug/sample/local")
    );
    assert_eq!(
        fixture.store.data_root(&sample, ManifestFormat::Claude),
        fixture.plugins().join("data/sample-debug")
    );
}

#[test]
fn agent_plugin_data_root_is_stable_and_unambiguous() {
    let fixture = fixture();
    let parent = fixture.plugins().join("data/agent-plugins");

    assert_eq!(
        fixture
            .store
            .data_root(&id("a-b", "c"), ManifestFormat::AgentPlugins),
        parent.join("6920dd17774030852d11d1b94758fcaae4f894c7b2f36301ed174bc3b33e0743")
    );
    assert_eq!(
        fixture
            .store
            .data_root(&id("a", "b-c"), ManifestFormat::AgentPlugins),
        parent.join("fa89b988ebbe54a68fdcbeb87fb913a5238d482084a3cee49a86288c2d45fa90")
    );
}

/// A marketplace at the fixture's `market` whose single entry has `entry`'s
/// fields and points at `./plugins/<name>`.
fn marketplace_with(fixture: &super::Fixture, name: &str, entry: &str) -> std::path::PathBuf {
    let root = fixture.path().join("market");
    write(
        &root.join(".agents/plugins/marketplace.json"),
        &format!(
            r#"{{"name": "debug", "plugins": [{{"name": "{name}", "source": "./plugins/{name}"{entry}}}]}}"#
        ),
    );
    root.join(".agents/plugins/marketplace.json")
}

#[test]
fn install_prefers_on_disk_manifest_version_over_fallback() {
    let fixture = fixture();
    let path = marketplace_with(&fixture, "sample-plugin", r#", "version": "9.9.9""#);
    write_plugin(
        &fixture.path().join("market/plugins"),
        "sample-plugin",
        "sample-plugin",
        Some("1.2.3-beta+7"),
    );

    let outcome = fixture
        .store
        .install_from_marketplace(&load_marketplace(&path).unwrap(), "sample-plugin")
        .unwrap();

    assert_eq!(outcome.version, "1.2.3-beta+7");
    assert!(outcome.root.join(".codex-plugin/plugin.json").is_file());
}

#[test]
fn install_stages_fallback_manifest_when_source_has_no_manifest() {
    let fixture = fixture();
    let path = marketplace_with(&fixture, "fallback-plugin", r#", "version": "1.2.3""#);
    write(
        &fixture
            .path()
            .join("market/plugins/fallback-plugin/skills/a/SKILL.md"),
        "skill",
    );
    let marketplace = load_marketplace(&path).unwrap();

    let outcome = fixture
        .store
        .install_from_marketplace(&marketplace, "fallback-plugin")
        .unwrap();

    assert_eq!(outcome.version, "1.2.3");
    assert_eq!(
        fs::read_to_string(outcome.root.join(".codex-plugin/plugin.json")).unwrap(),
        marketplace.plugins[0].fallback.contents
    );
    assert!(!fixture
        .path()
        .join("market/plugins/fallback-plugin/.codex-plugin")
        .exists());
}

#[test]
fn install_rejects_blank_manifest_version() {
    let fixture = fixture();
    let source = write_plugin(
        fixture.path(),
        "sample-plugin",
        "sample-plugin",
        Some("   "),
    );

    let error = fixture
        .store
        .install_directory(&source, &id("sample-plugin", "debug"))
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid plugin version in plugin.json: must not be blank"
    );
}

#[test]
fn agent_plugin_blank_version_uses_default_version() {
    let fixture = fixture();
    let source = fixture.path().join("agent-plugin");
    write(
        &source.join("plugin.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"agent-plugin","version":"   "}"#,
    );

    let outcome = fixture
        .store
        .install_directory(&source, &id("agent-plugin", "debug"))
        .unwrap();

    assert_eq!(outcome.version, "1.0.0");
}

#[test]
fn agent_plugin_unusable_version_becomes_a_digest() {
    let fixture = fixture();
    let source = fixture.path().join("agent-plugin");
    write(
        &source.join("plugin.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"agent-plugin","version":"release 7"}"#,
    );

    let outcome = fixture
        .store
        .install_directory(&source, &id("agent-plugin", "debug"))
        .unwrap();

    assert!(outcome.version.starts_with("agent-plugins-"));
    assert_eq!(outcome.version.len(), "agent-plugins-".len() + 24);
}

#[test]
fn agent_plugin_install_does_not_migrate_commands() {
    let fixture = fixture();
    let source = fixture.path().join("agent-plugin");
    write(
        &source.join("plugin.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"agent-plugin","commands":"./commands"}"#,
    );
    write(
        &source.join("commands/demo.md"),
        "---\ndescription: Demo\n---\n# Demo",
    );

    let outcome = fixture
        .store
        .install_directory(&source, &id("agent-plugin", "debug"))
        .unwrap();

    assert!(outcome.migrated_commands.is_empty());
    assert!(!migrated_command_skills_root(&outcome.root).exists());
}

#[test]
fn legacy_install_migrates_commands_into_skills() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "sample-plugin", "sample-plugin", None);
    write(
        &source.join("commands/review.md"),
        "---\ndescription: Review\n---\nReview it.\n",
    );

    let outcome = fixture
        .store
        .install_directory(&source, &id("sample-plugin", "debug"))
        .unwrap();

    assert_eq!(outcome.migrated_commands, vec!["source-command-review"]);
    assert!(migrated_command_skills_root(&outcome.root)
        .join("source-command-review/SKILL.md")
        .is_file());
    assert!(!migrated_command_skills_root(&source).exists());
}

#[test]
fn active_version_prefers_local_then_the_highest() {
    let fixture = fixture();
    let cache = fixture.plugins().join("cache/debug");
    let sample = id("sample-plugin", "debug");
    write_plugin(
        &cache,
        "sample-plugin/0123456789abcdef",
        "sample-plugin",
        None,
    );
    write_plugin(
        &cache,
        "sample-plugin/fedcba9876543210",
        "sample-plugin",
        None,
    );
    assert_eq!(
        fixture.store.active_version(&sample).as_deref(),
        Some("fedcba9876543210")
    );

    write_plugin(&cache, "sample-plugin/local", "sample-plugin", None);
    assert_eq!(
        fixture.store.active_version(&sample).as_deref(),
        Some("local")
    );
    assert_eq!(
        fixture.store.active_root(&sample),
        Some(cache.join("sample-plugin/local"))
    );
}

#[test]
fn active_version_compares_semver_versions_semantically() {
    let fixture = fixture();
    let cache = fixture.plugins().join("cache/debug");
    write_plugin(&cache, "sample-plugin/9.0.0", "sample-plugin", None);
    write_plugin(&cache, "sample-plugin/10.0.0", "sample-plugin", None);

    assert_eq!(
        fixture
            .store
            .active_version(&id("sample-plugin", "debug"))
            .as_deref(),
        Some("10.0.0")
    );
}

#[test]
fn install_with_new_version_keeps_existing_root_and_prunes_old_versions() {
    let fixture = fixture();
    let sample = id("sample-plugin", "debug");
    let v1 = write_plugin(fixture.path(), "v1", "sample-plugin", Some("1.0.0"));
    let v2 = write_plugin(fixture.path(), "v2", "sample-plugin", Some("2.0.0"));
    fixture.store.install_directory(&v1, &sample).unwrap();
    write(
        &fixture
            .store
            .data_root(&sample, ManifestFormat::Codex)
            .join("state"),
        "kept",
    );

    fixture.store.install_directory(&v2, &sample).unwrap();

    let base = fixture.plugins().join("cache/debug/sample-plugin");
    assert_eq!(
        fixture.store.active_version(&sample).as_deref(),
        Some("2.0.0")
    );
    assert!(base.join("2.0.0").is_dir());
    assert!(!base.join("1.0.0").exists());
    assert!(fixture
        .plugins()
        .join("data/sample-plugin-debug/state")
        .is_file());
}

#[test]
fn reinstalling_the_same_version_replaces_it() {
    let fixture = fixture();
    let sample = id("sample-plugin", "debug");
    let source = write_plugin(fixture.path(), "sample", "sample-plugin", Some("1.0.0"));
    fixture.store.install_directory(&source, &sample).unwrap();
    write(&source.join("skills/SKILL.md"), "changed");

    let outcome = fixture.store.install_directory(&source, &sample).unwrap();

    assert_eq!(
        fs::read_to_string(outcome.root.join("skills/SKILL.md")).unwrap(),
        "changed"
    );
    assert!(!fixture
        .plugins()
        .join(".staging")
        .read_dir()
        .unwrap()
        .any(|_| true));
}

#[test]
fn install_rejects_manifest_names_with_path_separators() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "source-dir", "../../etc", None);

    let error = fixture
        .store
        .install_directory(&source, &id("source-dir", "debug"))
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid plugin name: dots must separate non-empty name segments"
    );
}

#[test]
fn install_rejects_manifest_names_that_do_not_match_marketplace_plugin_name() {
    let fixture = fixture();
    let source = write_plugin(fixture.path(), "source-dir", "manifest-name", None);

    let error = fixture
        .store
        .install_directory(&source, &id("different-name", "debug"))
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "plugin.json name `manifest-name` does not match marketplace plugin name `different-name`"
    );
}

#[test]
fn install_without_a_manifest_fails() {
    let fixture = fixture();
    let source = fixture.path().join("empty");
    fs::create_dir_all(&source).unwrap();

    let error = fixture
        .store
        .install_directory(&source, &id("empty", "debug"))
        .unwrap_err();

    assert_eq!(error.to_string(), "missing plugin.json");
}

#[test]
fn the_install_record_keeps_the_version_source_and_content_hash() {
    let fixture = fixture();
    let sample = id("sample-plugin", "debug");
    let source = write_plugin(fixture.path(), "sample", "sample-plugin", Some("1.0.0"));

    let outcome = fixture.store.install_directory(&source, &sample).unwrap();

    let package = fixture.store.read(&sample).unwrap();
    let record = package.record.unwrap();
    assert_eq!(record.version.as_deref(), Some("1.0.0"));
    assert_eq!(record.format, Some(ManifestFormat::Codex));
    assert!(matches!(record.source, Some(PackageSource::Local { .. })));
    assert_eq!(record.content_hash, Some(outcome.content_hash.clone()));
    assert_eq!(content_hash(&outcome.root).unwrap(), outcome.content_hash);
    assert_eq!(package.issues, Vec::<String>::new());
    let file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.plugins().join("install.json")).unwrap())
            .unwrap();
    assert_eq!(file["packages"]["sample-plugin@debug"]["version"], "1.0.0");
}

#[test]
fn an_unreadable_install_record_is_quarantined_and_packages_stay_installed() {
    let fixture = fixture();
    let sample = id("sample-plugin", "debug");
    let source = write_plugin(fixture.path(), "sample", "sample-plugin", None);
    fixture.store.install_directory(&source, &sample).unwrap();
    fs::write(fixture.plugins().join("install.json"), "{not json").unwrap();

    let installed = fixture.store.installed();

    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].record, None);
    assert!(!fixture.plugins().join("install.json").exists());
    assert_eq!(
        fs::read_dir(fixture.plugins().join(".recovery"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn a_bad_record_field_or_entry_costs_only_itself() {
    let fixture = fixture();
    let sample = id("sample-plugin", "debug");
    let source = write_plugin(fixture.path(), "sample", "sample-plugin", None);
    fixture.store.install_directory(&source, &sample).unwrap();
    fs::write(
        fixture.plugins().join("install.json"),
        r#"{"packages": {"sample-plugin@debug": {"version": 3, "source": {"type": "ftp"},
            "contentHash": "sha256:00", "future": true}, "not an id": {}}, "future": 1}"#,
    )
    .unwrap();

    let record = fixture.store.read(&sample).unwrap().record.unwrap();

    assert_eq!(record.version, None);
    assert_eq!(record.source, None);
    assert_eq!(record.content_hash.as_deref(), Some("sha256:00"));
}

#[test]
fn uninstall_keeps_the_data_directory_unless_purged() {
    let fixture = fixture();
    let sample = id("sample-plugin", "debug");
    let source = write_plugin(fixture.path(), "sample", "sample-plugin", None);
    let data = fixture.store.data_root(&sample, ManifestFormat::Codex);
    fixture.store.install_directory(&source, &sample).unwrap();
    write(&data.join("state"), "kept");

    let outcome = fixture.store.uninstall(&sample, false).unwrap();

    assert!(outcome.removed);
    assert_eq!(outcome.purged_data, None);
    assert!(!fixture.plugins().join("cache/debug/sample-plugin").exists());
    assert!(data.join("state").is_file());
    assert!(fixture.store.read(&sample).is_err());
    assert!(fixture.store.installed().is_empty());

    fixture.store.install_directory(&source, &sample).unwrap();
    let outcome = fixture.store.uninstall(&sample, true).unwrap();
    assert_eq!(outcome.purged_data, Some(data.clone()));
    assert!(!data.exists());
    let record = fs::read_to_string(fixture.plugins().join("install.json")).unwrap();
    assert!(!record.contains("sample-plugin@debug"));
}

#[test]
fn opening_the_store_clears_what_an_interrupted_change_left() {
    let fixture = fixture();
    write(
        &fixture
            .plugins()
            .join(".staging/install-x/package/1.0.0/plugin.json"),
        "{}",
    );

    PackageStore::open(fixture.plugins(), ExecutorManager::default());

    assert!(!fixture.plugins().join(".staging").exists());
    assert!(fixture.store.installed().is_empty());
}

#[test]
fn the_content_hash_follows_the_sources_and_skips_python_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let root = write_plugin(directory.path(), "sample", "sample", None);
    let first = content_hash(&root).unwrap();
    assert!(first.starts_with("sha256:"));

    write(&root.join("__pycache__/x.cpython-312.pyc"), "bytecode");
    write(&root.join("tool.pyc"), "bytecode");
    write(&root.join(".venv/bin/python"), "venv");
    write(&root.join("dist.egg-info/PKG-INFO"), "info");
    assert_eq!(content_hash(&root).unwrap(), first);

    write(&root.join("skills/SKILL.md"), "changed");
    assert_ne!(content_hash(&root).unwrap(), first);
}

#[cfg(unix)]
mod symbolic_links {
    use std::fs;
    use std::os::unix::fs::symlink;

    use hanafubuki_agent_packages::PluginId;

    use crate::{fixture, write};

    #[test]
    fn install_rejects_symlinked_manifest_that_hides_lower_precedence_mcp_server() {
        let fixture = fixture();
        let source = fixture.path().join("manifest-switch");
        write(
            &source.join("benign.json"),
            r#"{"name":"manifest-switch","version":"1.2.3"}"#,
        );
        write(
            &source.join(".claude-plugin/plugin.json"),
            r#"{"name":"manifest-switch","version":"1.2.3","mcpServers":{"hidden":{"command":"/bin/sh"}}}"#,
        );
        fs::create_dir_all(source.join(".codex-plugin")).unwrap();
        symlink("../benign.json", source.join(".codex-plugin/plugin.json")).unwrap();

        let error = fixture
            .store
            .install_directory(&source, &PluginId::new("manifest-switch", "debug").unwrap())
            .unwrap_err();

        assert_eq!(error.to_string(), "missing plugin.json");
        assert!(!fixture
            .plugins()
            .join("cache/debug/manifest-switch")
            .exists());
    }

    #[test]
    fn install_rejects_symlinked_manifest_directory() {
        let fixture = fixture();
        let source = fixture.path().join("manifest-switch");
        let elsewhere = fixture.path().join("manifest-directory");
        write(
            &elsewhere.join("plugin.json"),
            r#"{"name":"manifest-switch"}"#,
        );
        write(
            &source.join(".claude-plugin/plugin.json"),
            r#"{"name":"manifest-switch"}"#,
        );
        symlink(&elsewhere, source.join(".codex-plugin")).unwrap();

        let error = fixture
            .store
            .install_directory(&source, &PluginId::new("manifest-switch", "debug").unwrap())
            .unwrap_err();

        assert_eq!(error.to_string(), "missing plugin.json");
    }

    #[test]
    fn agent_plugin_install_skips_symlinked_files() {
        let fixture = fixture();
        let source = fixture.path().join("agent-plugin");
        write(
            &source.join("plugin.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"agent-plugin"}"#,
        );
        let outside_skill = fixture.path().join("outside-SKILL.md");
        let outside_tool = fixture.path().join("outside-tool");
        write(&outside_skill, "---\nname: greet\n---\n");
        write(&outside_tool, "#!/bin/sh\n");
        fs::create_dir_all(source.join("skills/greet")).unwrap();
        fs::create_dir_all(source.join("bin")).unwrap();
        symlink(&outside_skill, source.join("skills/greet/SKILL.md")).unwrap();
        symlink(&outside_tool, source.join("bin/tool")).unwrap();

        let outcome = fixture
            .store
            .install_directory(&source, &PluginId::new("agent-plugin", "debug").unwrap())
            .unwrap();

        assert!(outcome.root.join("plugin.json").is_file());
        assert!(!outcome.root.join("skills/greet/SKILL.md").exists());
        assert!(!outcome.root.join("bin/tool").exists());
    }
}

#[test]
fn a_purge_finds_the_data_directory_of_a_package_whose_manifest_broke() {
    let fixture = fixture();
    let portable = id("agent-plugin", "debug");
    let source = fixture.path().join("agent-plugin");
    write(
        &source.join("plugin.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"agent-plugin"}"#,
    );
    let outcome = fixture.store.install_directory(&source, &portable).unwrap();
    let data = fixture
        .store
        .data_root(&portable, ManifestFormat::AgentPlugins);
    write(&data.join("state"), "data");
    fs::write(outcome.root.join("plugin.json"), "{broken").unwrap();

    assert_eq!(fixture.store.read(&portable).unwrap().data_root, data);
    assert_eq!(
        fixture
            .store
            .uninstall(&portable, true)
            .unwrap()
            .purged_data,
        Some(data.clone())
    );
    assert!(!data.exists());
}
