//! A package linked for development end to end: listed as linked, its
//! content read again once its directory changes while the `plugins` row
//! runs (no call reloads it), a broken manifest's repair too, never
//! updated, and unlinked with its directory kept.

use hanafubuki_agent_packages::PackageSource;

use super::*;
use crate::AgentPackageSource;

const ID: &str = "work@local";

/// A Claude Code plugin `work` under `paths` with the agent file
/// `reviewer`.
fn work_plugin(paths: &TestPaths) -> PathBuf {
    let root = paths.app_data_dir.join("sources").join("work");
    write(
        &root.join(".claude-plugin/plugin.json"),
        &json!({ "name": "work", "version": "0.1.0" }).to_string(),
    );
    write(
        &root.join("agents/reviewer.md"),
        "---\ndescription: Reviews.\ntools: Read\n---\nReview.\n",
    );
    root
}

const PLANNER: &str = "---\ndescription: Plans.\n---\nPlan.\n";

/// Writes `contents` at `path` until `condition` holds: the OS watch is
/// armed on its own thread, so a change made before it is lost, and made
/// again after a while.
async fn change_until(path: &Path, contents: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + TEST_WAIT;
    loop {
        write(path, contents);
        let retry = Instant::now() + Duration::from_secs(3);
        while Instant::now() < retry {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(Instant::now() < deadline, "{} was not read", path.display());
    }
}

fn plugin_roles(service: &AgentApplicationService) -> Vec<String> {
    service
        .roles()
        .roles()
        .map(|role| role.name.clone())
        .filter(|name| name.contains(':'))
        .collect()
}

async fn link(service: &AgentApplicationService, root: &Path) -> crate::AgentPackageInstall {
    service
        .install_package(AgentPackageSource::Linked {
            path: root.to_path_buf(),
            marketplace: None,
        })
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_packages_change_is_read_again_while_the_plugins_row_runs() {
    let (service, paths, _) = service_with_config("packages-linked-watch", |_, _| {});
    let root = work_plugin(&paths);
    let linked = link(&service, &root).await;
    assert_eq!(linked.package.id, ID);
    assert!(matches!(
        linked.package.source,
        Some(PackageSource::Linked { .. })
    ));
    assert_eq!(plugin_roles(&service), ["work:reviewer"]);
    service.start_plugins().await;

    let planner = || plugin_roles(&service).contains(&"work:planner".to_string());
    change_until(&root.join("agents/planner.md"), PLANNER, planner).await;
    // A manifest saved broken leaves the plugin without its roles, and its
    // directory still watched, so the repair is read.
    let manifest = root.join(".claude-plugin/plugin.json");
    let repaired = std::fs::read_to_string(&manifest).unwrap();
    change_until(&manifest, "{not json", || plugin_roles(&service).is_empty()).await;
    change_until(&manifest, &repaired, planner).await;
    let package = package(&service, ID).await;
    assert!(
        package.content.roles.contains(&"work:planner".to_string()),
        "{package:?}"
    );

    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_package_is_never_updated_and_unlinking_keeps_its_directory() {
    let (service, paths, config) = service_with_config("packages-linked-remove", |_, _| {});
    let root = work_plugin(&paths);
    link(&service, &root).await;

    let refused = service.update_package(ID).await.unwrap_err();
    assert!(refused.contains("never updated"), "{refused}");
    let installed = service
        .install_package(AgentPackageSource::Directory {
            path: root.clone(),
            marketplace: None,
        })
        .await
        .unwrap_err();
    assert!(installed.contains("is linked"), "{installed}");
    let package = package(&service, ID).await;
    assert!(matches!(package.source, Some(PackageSource::Linked { .. })));

    let removed = service.uninstall_package(ID, false).await.unwrap();
    assert_eq!(removed.id, ID);
    assert!(root.join(".claude-plugin/plugin.json").is_file());
    assert!(root.join("agents/reviewer.md").is_file());
    assert!(service.package(ID).await.unwrap().is_none());
    assert!(plugin_roles(&service).is_empty());
    let settings = config.settings().unwrap();
    assert!(!settings.plugins.packages.contains_key(ID));

    finish(service, paths).await;
}
