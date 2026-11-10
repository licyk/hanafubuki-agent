// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/marketplace_tests.rs` and the tests of
// `codex-rs/core-plugins/src/marketplace_add.rs` and
// `codex-rs/core-plugins/src/marketplace_remove.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: npm sources are skipped as unsupported;
// discovery from the home directory and the repository and the product
// policy are not ported; add and remove return records rather than writing
// the configuration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::{
    list_marketplaces, load_marketplace, InstallationPolicy, ManifestInterface,
    MarketplaceAddRequest, MarketplaceSourceType, PackageSource,
};
use serde_json::json;

use super::{fixture, write};

fn write_marketplace(root: &Path, relative: &str, contents: &serde_json::Value) -> PathBuf {
    let path = root.join(relative);
    write(&path, &contents.to_string());
    path
}

fn agents(root: &Path, contents: serde_json::Value) -> PathBuf {
    write_marketplace(root, ".agents/plugins/marketplace.json", &contents)
}

#[test]
fn finds_a_repository_marketplace_plugin_with_a_local_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = agents(
        root,
        json!({"name": "codex-curated", "plugins": [
            {"name": "local-plugin", "source": {"source": "local", "path": "./plugin-1"}}
        ]}),
    );

    let marketplace = load_marketplace(&path).unwrap();
    let plugin = marketplace.installable("local-plugin").unwrap();

    assert_eq!(plugin.id.key(), "local-plugin@codex-curated");
    assert_eq!(
        plugin.source,
        PackageSource::Local {
            path: root.join("plugin-1")
        }
    );
    assert_eq!(plugin.installation, InstallationPolicy::Available);
    assert!(!plugin.fallback.has_metadata);
}

#[test]
fn supports_the_claude_layout_and_a_string_local_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = write_marketplace(
        root,
        ".claude-plugin/marketplace.json",
        &json!({"name": "claude", "plugins": [{"name": "p", "source": "./plugins/p"}]}),
    );

    let plugin = load_marketplace(&path).unwrap().plugins.remove(0);

    assert_eq!(
        plugin.source,
        PackageSource::Local {
            path: root.join("plugins/p")
        }
    );
}

#[test]
fn supports_the_cursor_layout_and_a_bare_local_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = write_marketplace(
        root,
        ".cursor-plugin/marketplace.json",
        &json!({"name": "cursor", "plugins": [{"name": "p", "source": "plugins/p"}]}),
    );

    let plugin = load_marketplace(&path).unwrap().plugins.remove(0);

    assert_eq!(
        plugin.source,
        PackageSource::Local {
            path: root.join("plugins/p")
        }
    );
}

#[test]
fn supports_url_and_git_subdir_sources() {
    let directory = tempfile::tempdir().unwrap();
    let path = agents(
        directory.path(),
        json!({"name": "m", "plugins": [
            {"name": "a", "source": {"source": "git-subdir", "url": "owner/repo", "path": "./plugins/a", "ref": " main ", "sha": ""}},
            {"name": "b", "source": {"source": "url", "url": "https://github.com/owner/repo"}},
            {"name": "c", "source": {"source": "url", "url": "owner/repo.git"}},
            {"name": "d", "source": {"source": "url", "url": "git@github.com:owner/repo.git", "sha": "abc"}}
        ]}),
    );

    let sources = load_marketplace(&path)
        .unwrap()
        .plugins
        .into_iter()
        .map(|plugin| plugin.source)
        .collect::<Vec<_>>();

    let git = |url: &str, path: Option<&str>, ref_name: Option<&str>, sha: Option<&str>| {
        PackageSource::Git {
            url: url.to_string(),
            path: path.map(str::to_string),
            ref_name: ref_name.map(str::to_string),
            sha: sha.map(str::to_string),
        }
    };
    assert_eq!(
        sources,
        vec![
            git(
                "https://github.com/owner/repo.git",
                Some("plugins/a"),
                Some("main"),
                None
            ),
            git("https://github.com/owner/repo.git", None, None, None),
            git("https://github.com/owner/repo.git", None, None, None),
            git("git@github.com:owner/repo.git", None, None, Some("abc")),
        ]
    );
}

#[test]
fn relative_git_urls_resolve_against_the_marketplace_root_and_stay_in_it() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = agents(
        root,
        json!({"name": "m", "plugins": [
            {"name": "inside", "source": {"source": "url", "url": "./repos/plugin.git"}},
            {"name": "outside", "source": {"source": "url", "url": "../elsewhere.git"}}
        ]}),
    );

    let marketplace = load_marketplace(&path).unwrap();

    assert_eq!(marketplace.plugins.len(), 1);
    assert_eq!(
        marketplace.plugins[0].source,
        PackageSource::Git {
            url: root.join("repos").join("plugin.git").display().to_string(),
            path: None,
            ref_name: None,
            sha: None,
        }
    );
    assert_eq!(marketplace.issues.len(), 1);
}

#[test]
fn unsupported_and_invalid_entries_are_skipped_and_the_marketplace_kept() {
    let directory = tempfile::tempdir().unwrap();
    let path = agents(
        directory.path(),
        json!({"name": "m", "plugins": [
            {"name": "npm", "source": {"source": "npm", "package": "@scope/p"}},
            {"name": "github", "source": {"source": "github", "repo": "owner/repo"}},
            {"name": "root-subdir", "source": {"source": "git-subdir", "url": "owner/repo", "path": "./"}},
            {"name": "escape", "source": "./../outside"},
            {"name": "no-dot", "source": "plugins/x"},
            {"name": "bad name!", "source": "./p"},
            {"source": "./p"},
            "not an object",
            {"name": "kept", "source": "./kept"}
        ]}),
    );

    let marketplace = load_marketplace(&path).unwrap();

    let names = marketplace
        .plugins
        .iter()
        .map(|plugin| plugin.id.plugin_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["kept"]);
    assert_eq!(marketplace.issues.len(), 8, "{:?}", marketplace.issues);
}

#[test]
fn builds_the_manifest_fallback_from_the_entry() {
    let directory = tempfile::tempdir().unwrap();
    let path = agents(
        directory.path(),
        json!({"name": "m", "plugins": [{
            "name": "p",
            "source": {"source": "url", "url": "owner/repo"},
            "category": "Productivity",
            "description": "From the entry",
            "version": "1.0.0",
            "displayName": "Entry Plugin",
            "author": {"name": " Entry Author "},
            "homepage": "https://example.com",
            "interface": {"logo": "./logo.svg"},
            "policy": {"installation": "INSTALLED_BY_DEFAULT"}
        }]}),
    );

    let plugin = load_marketplace(&path).unwrap().plugins.remove(0);

    assert!(plugin.fallback.has_metadata);
    let fallback: serde_json::Value = serde_json::from_str(&plugin.fallback.contents).unwrap();
    assert_eq!(
        fallback,
        json!({
            "name": "p",
            "category": "Productivity",
            "description": "From the entry",
            "version": "1.0.0",
            "displayName": "Entry Plugin",
            "author": {"name": " Entry Author "},
            "homepage": "https://example.com",
            "interface": {
                "logo": "./logo.svg",
                "displayName": "Entry Plugin",
                "developerName": "Entry Author",
                "websiteUrl": "https://example.com",
                "category": "Productivity"
            }
        })
    );
    assert_eq!(plugin.installation, InstallationPolicy::InstalledByDefault);
    assert_eq!(plugin.local_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        plugin.interface,
        Some(ManifestInterface {
            display_name: Some("Entry Plugin".to_string()),
            developer_name: Some("Entry Author".to_string()),
            website_url: Some("https://example.com".to_string()),
            category: Some("Productivity".to_string()),
            ..ManifestInterface::default()
        })
    );
}

#[test]
fn a_local_sources_own_manifest_describes_it_and_the_category_wins() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = agents(
        root,
        json!({"name": "m", "plugins": [
            {"name": "p", "source": "./p", "category": "Marketplace category"}
        ]}),
    );
    write(
        &root.join("p/.codex-plugin/plugin.json"),
        r#"{"name": "p", "version": "3.0.0", "keywords": ["k"],
            "interface": {"category": "Own", "logo": "./logo.svg", "composerIcon": "logo.svg"}}"#,
    );

    let plugin = load_marketplace(&path).unwrap().plugins.remove(0);

    assert_eq!(plugin.local_version.as_deref(), Some("3.0.0"));
    assert_eq!(plugin.keywords, vec!["k"]);
    let interface = plugin.interface.unwrap();
    assert_eq!(interface.category.as_deref(), Some("Marketplace category"));
    assert_eq!(interface.logo, Some(root.join("p/logo.svg")));
    assert_eq!(interface.composer_icon, None);
}

#[test]
fn reports_a_missing_or_unavailable_plugin_and_uses_the_first_duplicate() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = agents(
        root,
        json!({"name": "m", "plugins": [
            {"name": "dup", "source": "./first"},
            {"name": "dup", "source": "./second"},
            {"name": "off", "source": "./off", "policy": {"installation": "NOT_AVAILABLE"}}
        ]}),
    );
    let marketplace = load_marketplace(&path).unwrap();

    assert_eq!(
        marketplace.installable("dup").unwrap().source,
        PackageSource::Local {
            path: root.join("first")
        }
    );
    assert_eq!(
        marketplace.installable("missing").unwrap_err().to_string(),
        "plugin `missing` was not found in marketplace `m`"
    );
    assert_eq!(
        marketplace.installable("off").unwrap_err().to_string(),
        "plugin `off` is not available for install in marketplace `m`"
    );
}

#[test]
fn list_prefers_the_first_supported_layout_and_reads_the_display_name() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    agents(
        root,
        json!({"name": "agents", "interface": {"displayName": "Agents"}, "plugins": []}),
    );
    write_marketplace(
        root,
        ".claude-plugin/marketplace.json",
        &json!({"name": "claude", "plugins": []}),
    );

    let listing = list_marketplaces(&[root.to_path_buf()]);

    assert_eq!(listing.marketplaces.len(), 1);
    assert_eq!(listing.marketplaces[0].name, "agents");
    assert_eq!(
        listing.marketplaces[0].display_name.as_deref(),
        Some("Agents")
    );
}

#[test]
fn list_supports_the_api_marketplace_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write_marketplace(
        root,
        ".agents/plugins/api_marketplace.json",
        &json!({"name": "api", "plugins": [{"name": "p", "source": "./p"}]}),
    );

    let listing = list_marketplaces(&[root.to_path_buf()]);

    assert_eq!(listing.marketplaces[0].name, "api");
    assert_eq!(listing.marketplaces[0].root, root);
}

#[test]
fn list_keeps_distinct_marketplaces_of_the_same_name_and_reports_errors() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    let broken = directory.path().join("broken");
    let unnamed = directory.path().join("unnamed");
    agents(&first, json!({"name": "same", "plugins": []}));
    agents(&second, json!({"name": "same", "plugins": []}));
    write(&broken.join(".agents/plugins/marketplace.json"), "{");
    agents(&unnamed, json!({"name": "dotted.name", "plugins": []}));

    let listing = list_marketplaces(&[
        first.clone(),
        second.clone(),
        first.clone(),
        broken,
        unnamed,
        directory.path().join("missing"),
    ]);

    let roots = listing
        .marketplaces
        .iter()
        .map(|marketplace| marketplace.root.clone())
        .collect::<Vec<_>>();
    assert_eq!(roots, vec![first, second]);
    assert_eq!(listing.errors.len(), 3, "{:?}", listing.errors);
}

#[test]
fn adding_a_local_marketplace_records_it_where_it_is() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    agents(&source, json!({"name": "debug", "plugins": []}));
    let request = MarketplaceAddRequest {
        source: source.display().to_string(),
        ..Default::default()
    };

    let added = fixture
        .store
        .add_marketplace(&request, &BTreeMap::new())
        .unwrap();

    assert_eq!(added.name, "debug");
    assert!(!added.already_added);
    assert_eq!(added.record.source_type, MarketplaceSourceType::Local);
    assert_eq!(
        ResolvedPath::resolve(&added.root).unwrap(),
        ResolvedPath::resolve(&source).unwrap()
    );
    assert!(added.record.last_updated.is_some());
    assert!(!fixture.plugins().join("marketplaces").exists());

    let configured = BTreeMap::from([(added.name.clone(), added.record.clone())]);
    let again = fixture
        .store
        .add_marketplace(&request, &configured)
        .unwrap();
    assert!(again.already_added);
    assert_eq!(again.record, added.record);

    let other = fixture.path().join("other");
    agents(&other, json!({"name": "debug", "plugins": []}));
    let error = fixture
        .store
        .add_marketplace(
            &MarketplaceAddRequest {
                source: other.display().to_string(),
                ..Default::default()
            },
            &configured,
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("already added from a different source"));
}

#[test]
fn a_local_marketplace_takes_neither_a_ref_nor_sparse_paths() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    agents(&source, json!({"name": "debug", "plugins": []}));

    let sparse = fixture.store.add_marketplace(
        &MarketplaceAddRequest {
            source: source.display().to_string(),
            ref_name: None,
            sparse_paths: vec![".agents".to_string()],
        },
        &BTreeMap::new(),
    );
    let with_ref = fixture.store.add_marketplace(
        &MarketplaceAddRequest {
            source: source.display().to_string(),
            ref_name: Some("main".to_string()),
            sparse_paths: Vec::new(),
        },
        &BTreeMap::new(),
    );

    assert_eq!(
        sparse.unwrap_err().to_string(),
        "--sparse is only supported for git marketplace sources"
    );
    assert_eq!(
        with_ref.unwrap_err().to_string(),
        "--ref is only supported for git marketplace sources"
    );
}

#[test]
fn removing_a_marketplace_deletes_its_checkout_only() {
    let fixture = fixture();
    let checkout = fixture.store.marketplace_checkout("debug");
    agents(&checkout, json!({"name": "debug", "plugins": []}));

    assert_eq!(
        fixture.store.remove_marketplace("debug", None).unwrap(),
        Some(checkout.clone())
    );
    assert!(!checkout.exists());
    assert_eq!(
        fixture
            .store
            .remove_marketplace("missing", None)
            .unwrap_err()
            .to_string(),
        "marketplace `missing` is not configured or installed"
    );
    assert!(fixture.store.remove_marketplace("../x", None).is_err());
}
