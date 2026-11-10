//! Codex's, Claude Code's, Agent Plugins' and this project's packages
//! install, list and uninstall from a local marketplace and a git one; a
//! git marketplace upgrades when its remote moves, and a package from a
//! git source is cloned sparsely.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hanafubuki_agent_packages::{
    ManifestFormat, MarketplaceAddRequest, MarketplaceRecord, MarketplaceSourceType,
    MarketplaceUpgrade, PackageSource, PackageStore,
};
use serde_json::{json, Value};

use super::{bare_repository, commit, copy_dir, fixture, fixtures, setup_git, write};

/// Every installable package of the fixture marketplace.
const PACKAGES: [(&str, &str, ManifestFormat); 5] = [
    ("claude-demo", "1.0.0", ManifestFormat::Claude),
    ("codex-demo", "2.1.0", ManifestFormat::Codex),
    ("fallback-demo", "0.3.0", ManifestFormat::Codex),
    ("hanafubuki-demo", "0.1.0", ManifestFormat::Hanafubuki),
    ("portable-demo", "1.0.0", ManifestFormat::AgentPlugins),
];

fn install_list_and_uninstall_every_package(
    store: &PackageStore,
    configured: &BTreeMap<String, MarketplaceRecord>,
) {
    let listing = store.configured_marketplaces(configured);
    assert_eq!(listing.errors, Vec::new());
    let marketplace = &listing.marketplaces[0];
    assert_eq!(marketplace.name, "demo-market");
    assert_eq!(
        marketplace.display_name.as_deref(),
        Some("Demo marketplace")
    );
    assert_eq!(marketplace.issues.len(), 1, "{:?}", marketplace.issues);
    assert!(store
        .install_from_marketplace(marketplace, "unavailable-demo")
        .is_err());

    for (name, version, _) in PACKAGES {
        let outcome = store.install_from_marketplace(marketplace, name).unwrap();
        assert_eq!(outcome.version, version, "{name}");
    }

    let installed = store.installed();
    let listed = installed
        .iter()
        .map(|package| {
            let format = package.manifest.as_ref().unwrap().format;
            (
                package.id.plugin_name.as_str(),
                package.version.as_str(),
                format,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(listed, PACKAGES.to_vec());
    for package in &installed {
        assert_eq!(package.id.marketplace_name, "demo-market");
        assert_eq!(package.issues, Vec::<String>::new(), "{}", package.id);
        assert!(package.record.as_ref().unwrap().content_hash.is_some());
    }
    let claude = &installed[0];
    assert!(claude
        .root
        .join(".codex-plugin/migrated-command-skills/source-command-review/SKILL.md")
        .is_file());
    assert!(claude.root.join("hooks/hooks.json").is_file());
    assert_eq!(
        claude.data_root,
        store.root().join("data/claude-demo-demo-market")
    );
    let portable = &installed[4];
    assert!(portable
        .data_root
        .starts_with(store.root().join("data/agent-plugins")));
    assert!(!portable.root.join(".codex-plugin").exists());

    for package in installed {
        assert!(store.uninstall(&package.id, false).unwrap().removed);
    }
    assert!(store.installed().is_empty());
}

#[test]
fn packages_install_list_and_uninstall_from_a_local_marketplace() {
    let fixture = fixture();
    let source = fixture.path().join("market");
    copy_dir(&fixtures().join("marketplace"), &source);

    let added = fixture
        .store
        .add_marketplace(
            &MarketplaceAddRequest {
                source: source.display().to_string(),
                ..Default::default()
            },
            &BTreeMap::new(),
        )
        .unwrap();

    assert_eq!(added.name, "demo-market");
    let configured = BTreeMap::from([(added.name, added.record)]);
    install_list_and_uninstall_every_package(&fixture.store, &configured);
}

#[test]
fn packages_install_list_and_uninstall_from_a_git_marketplace() {
    let fixture = fixture();
    let repositories = fixture.path().join("repositories");
    let remote_package = repositories.join("packages-work");
    write(
        &remote_package.join("packages/remote-demo/.claude-plugin/plugin.json"),
        r#"{"name": "remote-demo", "version": "4.0.0"}"#,
    );
    write(
        &remote_package.join("unrelated/large.bin"),
        "not checked out",
    );
    let packages_url = bare_repository(
        &remote_package,
        &repositories.join("packages-clone"),
        &repositories.join("packages.git"),
    );
    let work = repositories.join("market-work");
    copy_dir(&fixtures().join("marketplace"), &work);
    add_entry(
        &work,
        json!({"name": "remote-demo", "source": {"source": "git-subdir", "url": packages_url, "path": "packages/remote-demo"}}),
    );
    let url = bare_repository(
        &work,
        &repositories.join("market-clone"),
        &repositories.join("market.git"),
    );

    let added = fixture
        .store
        .add_marketplace(
            &MarketplaceAddRequest {
                source: url.clone(),
                ..Default::default()
            },
            &BTreeMap::new(),
        )
        .unwrap();

    assert_eq!(added.name, "demo-market");
    assert_eq!(
        added.root,
        fixture.plugins().join("marketplaces/demo-market")
    );
    assert_eq!(added.record.source_type, MarketplaceSourceType::Git);
    assert_eq!(added.record.source, url);
    assert!(added.record.last_revision.is_some());
    let configured = BTreeMap::from([(added.name.clone(), added.record.clone())]);

    let marketplace = fixture
        .store
        .configured_marketplaces(&configured)
        .marketplaces
        .remove(0);
    let remote = fixture
        .store
        .install_from_marketplace(&marketplace, "remote-demo")
        .unwrap();
    assert_eq!(remote.version, "4.0.0");
    assert!(remote.root.join(".claude-plugin/plugin.json").is_file());
    assert!(!remote.root.join(".git").exists());
    let record = fixture.store.read(&remote.id).unwrap().record.unwrap();
    assert!(matches!(record.source, Some(PackageSource::Git { .. })));
    fixture.store.uninstall(&remote.id, false).unwrap();

    install_list_and_uninstall_every_package(&fixture.store, &configured);
    fixture.assert_no_process_left();
}

fn add_entry(marketplace: &Path, entry: Value) {
    let path = marketplace.join(".claude-plugin/marketplace.json");
    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    manifest["plugins"].as_array_mut().unwrap().push(entry);
    fs::write(&path, manifest.to_string()).unwrap();
}

#[test]
fn a_git_marketplace_upgrades_when_its_remote_moves() {
    let fixture = fixture();
    let repositories = fixture.path().join("repositories");
    let work = repositories.join("work");
    let bare = repositories.join("market.git");
    let url = bare_repository(&fixtures().join("marketplace"), &work, &bare);
    setup_git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
    let added = fixture
        .store
        .add_marketplace(
            &MarketplaceAddRequest {
                source: url,
                ..Default::default()
            },
            &BTreeMap::new(),
        )
        .unwrap();

    assert_eq!(
        fixture
            .store
            .upgrade_marketplace("demo-market", &added.record)
            .unwrap(),
        MarketplaceUpgrade::UpToDate
    );

    add_entry(
        &work,
        json!({"name": "later", "source": "./plugins/codex-demo"}),
    );
    commit(&work, "add later");
    setup_git(&work, &["push", "-q", "origin", "main"]);
    let MarketplaceUpgrade::Upgraded(upgraded) = fixture
        .store
        .upgrade_marketplace("demo-market", &added.record)
        .unwrap()
    else {
        panic!("the marketplace should upgrade");
    };
    assert_ne!(upgraded.last_revision, added.record.last_revision);
    assert_eq!(
        upgraded.last_revision.as_deref(),
        Some(setup_git(&work, &["rev-parse", "HEAD"]).as_str())
    );
    let checkout = fixture.plugins().join("marketplaces/demo-market");
    assert!(
        fs::read_to_string(checkout.join(".claude-plugin/marketplace.json"))
            .unwrap()
            .contains("later")
    );
    assert_eq!(
        fixture
            .store
            .upgrade_marketplace("demo-market", &upgraded)
            .unwrap(),
        MarketplaceUpgrade::UpToDate
    );

    let manifest = work.join(".claude-plugin/marketplace.json");
    let renamed = fs::read_to_string(&manifest)
        .unwrap()
        .replace("demo-market", "renamed");
    fs::write(&manifest, renamed).unwrap();
    commit(&work, "rename");
    setup_git(&work, &["push", "-q", "origin", "main"]);
    let error = fixture
        .store
        .upgrade_marketplace("demo-market", &upgraded)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "upgraded marketplace name `renamed` does not match configured marketplace `demo-market`"
    );
    assert!(
        fs::read_to_string(checkout.join(".claude-plugin/marketplace.json"))
            .unwrap()
            .contains("later")
    );

    assert_eq!(
        fixture
            .store
            .remove_marketplace("demo-market", Some(&upgraded))
            .unwrap(),
        Some(checkout.clone())
    );
    assert!(!checkout.exists());
    fixture.assert_no_process_left();
}

#[test]
fn a_git_marketplace_can_check_out_sparse_paths_at_a_ref() {
    let fixture = fixture();
    let repositories = fixture.path().join("repositories");
    let work = repositories.join("work");
    let url = bare_repository(
        &fixtures().join("marketplace"),
        &work,
        &repositories.join("m.git"),
    );

    let added = fixture
        .store
        .add_marketplace(
            &MarketplaceAddRequest {
                source: format!("{url}#main"),
                ref_name: None,
                sparse_paths: vec![
                    ".claude-plugin".to_string(),
                    "plugins/codex-demo".to_string(),
                ],
            },
            &BTreeMap::new(),
        )
        .unwrap();

    assert_eq!(added.record.ref_name.as_deref(), Some("main"));
    let checkout = &added.root;
    assert!(checkout.join(".claude-plugin/marketplace.json").is_file());
    assert!(checkout
        .join("plugins/codex-demo/.codex-plugin/plugin.json")
        .is_file());
    assert!(!checkout.join("plugins/claude-demo").exists());

    let configured = BTreeMap::from([(added.name.clone(), added.record.clone())]);
    let again = fixture
        .store
        .add_marketplace(
            &MarketplaceAddRequest {
                source: url,
                ref_name: Some("main".to_string()),
                sparse_paths: vec![
                    ".claude-plugin".to_string(),
                    "plugins/codex-demo".to_string(),
                ],
            },
            &configured,
        )
        .unwrap();
    assert!(again.already_added);
    fixture.assert_no_process_left();
}
