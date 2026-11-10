//! The lifecycle calls end to end: a marketplace added from a directory and
//! one cloned from a git repository, a plugin installed from each and from
//! a directory, turned off and on, updated, retried and uninstalled, each
//! change told to the plugin event sink; a git marketplace upgraded once its
//! remote moved.

use std::process::Command;
use std::sync::Arc;

use hanafubuki_agent_packages::MarketplaceAddRequest;
use parking_lot::Mutex;

use super::*;
use crate::{AgentPackageChange, AgentPackageChangeKind, AgentPackageSource, AgentPluginEventSink};

#[derive(Default)]
struct RecordedChanges(Mutex<Vec<AgentPackageChange>>);

impl AgentPluginEventSink for RecordedChanges {
    fn plugins_changed(&self) {}

    fn package_changed(&self, change: &AgentPackageChange) {
        self.0.lock().push(change.clone());
    }
}

impl RecordedChanges {
    fn kinds(&self) -> Vec<(String, AgentPackageChangeKind)> {
        self.0
            .lock()
            .iter()
            .map(|change| (change.id.clone(), change.kind))
            .collect()
    }
}

/// A marketplace `name` at `root` offering the Claude Code plugin `demo`
/// (a Skill and a Stop hook) at `plugins/demo`, at `version`.
fn marketplace(root: &Path, name: &str, version: &str) {
    write(
        &root.join(".claude-plugin/marketplace.json"),
        &json!({
            "name": name,
            "plugins": [{ "name": "demo", "source": "./plugins/demo", "description": "A demo." }],
        })
        .to_string(),
    );
    let plugin = root.join("plugins/demo");
    write(
        &plugin.join(".claude-plugin/plugin.json"),
        &json!({ "name": "demo", "version": version, "description": "The demo plugin." })
            .to_string(),
    );
    write(
        &plugin.join("skills/search/SKILL.md"),
        "---\nname: search\ndescription: Searches the demo records.\n---\nSearch.\n",
    );
    write(
        &plugin.join("hooks/hooks.json"),
        &json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "true" }] }] } })
            .to_string(),
    );
}

fn changes(service: &mut AgentApplicationService) -> Arc<RecordedChanges> {
    let changes = Arc::new(RecordedChanges::default());
    service.test_replace_plugin_events(changes.clone());
    changes
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_marketplaces_plugin_is_installed_turned_off_and_on_updated_and_uninstalled() {
    let (mut service, paths, _) = service_with_config("packages-manage-local", |_, _| {});
    let changes = changes(&mut service);
    let root = paths.app_data_dir.join("sources/market");
    marketplace(&root, "demo-market", "1.0.0");

    let added = service
        .add_marketplace(MarketplaceAddRequest {
            source: root.display().to_string(),
            ..MarketplaceAddRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(added.name, "demo-market");
    assert!(!added.already_added);
    let config_file = paths.config_dir.join("config.toml");
    let marketplaces = service.marketplaces().await.unwrap();
    let [offered] = marketplaces.as_slice() else {
        panic!("one marketplace: {marketplaces:?}");
    };
    assert_eq!(offered.plugins.len(), 1);
    assert!(offered.plugins[0].installable && !offered.plugins[0].installed);

    let install = service
        .install_package(AgentPackageSource::Marketplace {
            marketplace: "demo-market".to_string(),
            plugin: "demo".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(install.package.id, "demo@demo-market");
    assert!(install.package.enabled);
    assert_eq!(install.package.version, "1.0.0");
    assert_eq!(install.hooks_to_trust.len(), 1);
    assert!(install.permissions.is_empty());
    assert_eq!(
        install.package.hooks_to_trust,
        [install.hooks_to_trust[0].key.clone()]
    );
    assert_eq!(
        plugin_skill_ids(&service),
        ["plugin:demo@demo-market:search"]
    );
    let written = std::fs::read_to_string(&config_file).unwrap();
    assert!(written.contains("enabled = true"), "{written}");
    assert!(service.marketplaces().await.unwrap()[0].plugins[0].enabled);

    // The user's overlay on the package's servers outlives its toggles.
    let overlay = "\n[plugins.\"demo@demo-market\".mcpServers.records]\nenabledTools = [\"a\"]\n";
    std::fs::write(&config_file, written + overlay).unwrap();
    let off = service
        .set_package_enabled("demo@demo-market", false)
        .await
        .unwrap();
    assert!(!off.enabled);
    assert!(plugin_skill_ids(&service).is_empty());
    let on = service
        .set_package_enabled("demo@demo-market", true)
        .await
        .unwrap();
    assert!(on.enabled);
    assert_eq!(
        plugin_skill_ids(&service),
        ["plugin:demo@demo-market:search"]
    );
    assert!(std::fs::read_to_string(&config_file)
        .unwrap()
        .contains("enabledTools = [\"a\"]"));

    service
        .trust_hooks(install.package.hooks_to_trust.clone())
        .await
        .unwrap();
    assert!(package(&service, "demo@demo-market")
        .await
        .hooks_to_trust
        .is_empty());
    marketplace(&root, "demo-market", "1.1.0");
    let updated = service.update_package("demo@demo-market").await.unwrap();
    assert_eq!(updated.package.version, "1.1.0");
    assert!(updated.package.enabled);
    // A changed package's hooks wait to be trusted again.
    assert_eq!(updated.hooks_to_trust.len(), 1);
    let retried = service.retry_package("demo@demo-market").await.unwrap();
    assert!(retried.issues.is_empty(), "{:?}", retried.issues);

    let removed = service
        .uninstall_package("demo@demo-market", true)
        .await
        .unwrap();
    assert!(removed.purged_data.is_some_and(|data| !data.exists()));
    assert!(service.packages().await.unwrap().is_empty());
    assert!(plugin_skill_ids(&service).is_empty());
    assert!(!std::fs::read_to_string(&config_file)
        .unwrap()
        .contains("demo@demo-market"));
    let error = service
        .uninstall_package("demo@demo-market", false)
        .await
        .unwrap_err();
    assert!(error.contains("is not installed"), "{error}");

    service.remove_marketplace("demo-market").await.unwrap();
    assert!(service.marketplaces().await.unwrap().is_empty());
    let id = "demo@demo-market".to_string();
    assert_eq!(
        changes.kinds(),
        [
            (id.clone(), AgentPackageChangeKind::Installed),
            (id.clone(), AgentPackageChangeKind::Disabled),
            (id.clone(), AgentPackageChangeKind::Enabled),
            (id.clone(), AgentPackageChangeKind::Updated),
            (id.clone(), AgentPackageChangeKind::Removed),
            (id, AgentPackageChangeKind::Failed),
        ]
    );
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_is_installed_as_local_and_a_marketplace_turned_off_keeps_its_plugins_off() {
    let (mut service, paths, config) = service_with_config("packages-manage-directory", |_, _| {});
    let changes = changes(&mut service);
    let source = claude_plugin(&paths, "solo", json!({}), None);

    let install = service
        .install_package(AgentPackageSource::Directory {
            path: source,
            marketplace: None,
        })
        .await
        .unwrap();
    assert_eq!(install.package.id, "solo@local");
    assert_eq!(plugin_skill_ids(&service), ["plugin:solo@local:search"]);
    assert!(install.hooks_to_trust.is_empty());

    let record = hanafubuki_agent_packages::MarketplaceRecord {
        source_type: hanafubuki_agent_packages::MarketplaceSourceType::Local,
        source: paths.app_data_dir.display().to_string(),
        ref_name: None,
        sparse_paths: None,
        last_updated: None,
        last_revision: None,
        enabled: Some(false),
    };
    config.set_marketplace("local", Some(&record)).unwrap();
    service.reload_packages().await.unwrap();
    assert!(!package(&service, "solo@local").await.enabled);
    let error = service
        .set_package_enabled("solo@local", true)
        .await
        .unwrap_err();
    assert!(
        error.contains("its marketplace `local` is turned off"),
        "{error}"
    );
    config.set_marketplace("local", None).unwrap();
    // Installed from a directory, it updates from there.
    let updated = service.update_package("solo@local").await.unwrap();
    assert!(updated.package.enabled);

    let error = service
        .set_package_enabled("missing@local", false)
        .await
        .unwrap_err();
    assert!(error.contains("is not installed"), "{error}");
    assert_eq!(
        changes.kinds(),
        [
            ("solo@local".to_string(), AgentPackageChangeKind::Installed),
            ("solo@local".to_string(), AgentPackageChangeKind::Failed),
            ("solo@local".to_string(), AgentPackageChangeKind::Updated),
            ("missing@local".to_string(), AgentPackageChangeKind::Failed),
        ]
    );
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_git_marketplace_is_cloned_installed_from_and_upgraded_once_its_remote_moved() {
    let (service, paths, _) = service_with_config("packages-manage-git", |_, _| {});
    let contents = paths.app_data_dir.join("sources/git-market");
    marketplace(&contents, "git-market", "1.0.0");
    let work = paths.app_data_dir.join("git/work");
    let url = bare_repository(&contents, &work, &paths.app_data_dir.join("git/remote.git"));

    let added = service
        .add_marketplace(MarketplaceAddRequest {
            source: url.clone(),
            ..MarketplaceAddRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(added.name, "git-market");
    assert!(added.root.starts_with(plugins_root(&paths)));
    let again = service
        .add_marketplace(MarketplaceAddRequest {
            source: url,
            ..MarketplaceAddRequest::default()
        })
        .await
        .unwrap();
    assert!(again.already_added);
    let first_revision = service.marketplaces().await.unwrap()[0]
        .last_revision
        .clone()
        .unwrap();
    let install = service
        .install_package(AgentPackageSource::Marketplace {
            marketplace: "git-market".to_string(),
            plugin: "demo".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(install.package.version, "1.0.0");

    let upgrades = service.upgrade_marketplace(None).await.unwrap();
    assert_eq!(upgrades.len(), 1);
    assert!(!upgrades[0].upgraded, "{upgrades:?}");
    marketplace(&work, "git-market", "2.0.0");
    commit(&work, "version 2");
    git(&work, &["push", "-q", "origin", "main"]);
    let upgrades = service
        .upgrade_marketplace(Some("git-market".to_string()))
        .await
        .unwrap();
    assert!(upgrades[0].upgraded, "{upgrades:?}");
    let marketplaces = service.marketplaces().await.unwrap();
    assert_ne!(
        marketplaces[0].last_revision.as_ref(),
        Some(&first_revision)
    );
    assert_eq!(marketplaces[0].plugins[0].version.as_deref(), Some("2.0.0"));
    // The plugin stays at its version until it is updated.
    assert_eq!(package(&service, "demo@git-market").await.version, "1.0.0");
    let updated = service.update_package("demo@git-market").await.unwrap();
    assert_eq!(updated.package.version, "2.0.0");

    service.remove_marketplace("git-market").await.unwrap();
    assert!(!added.root.exists());
    assert!(service.marketplaces().await.unwrap().is_empty());
    // Its plugin stays installed, and cannot be updated without it.
    assert!(service.update_package("demo@git-market").await.is_err());
    finish(service, paths).await;
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

fn commit(work: &Path, message: &str) {
    git(work, &["add", "-A"]);
    git(
        work,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

/// A bare repository at `bare` holding one commit of `contents`, which the
/// work tree `work` pushes to as `origin`; its `file://` URL.
fn bare_repository(contents: &Path, work: &Path, bare: &Path) -> String {
    std::fs::create_dir_all(work).unwrap();
    copy_dir(contents, work);
    git(work, &["init", "-q", "-b", "main"]);
    git(work, &["config", "core.autocrlf", "false"]);
    commit(work, "initial");
    std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
    git(
        bare.parent().unwrap(),
        &[
            "clone",
            "-q",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    git(work, &["remote", "add", "origin", bare.to_str().unwrap()]);
    let path = bare.to_str().unwrap().replace('\\', "/");
    format!("file:///{}", path.trim_start_matches('/'))
}

fn copy_dir(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}
