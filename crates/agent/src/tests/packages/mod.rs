//! Installed plugins end to end: a package installed into the store and
//! turned on in `[plugins]` contributes its Skills, MCP servers and hooks
//! after a reload, and stops once turned off, without a restart; the
//! user's MCP server and an earlier plugin come first for a server name;
//! an Agent Plugins package's Skills are its root's direct children and its
//! hooks never run; a broken package costs only itself.

use std::path::{Path, PathBuf};

use hanafubuki_agent_packages::{PackageStore, PluginId};
use hanafubuki_agent_process::executor::ExecutorManager;

use super::*;

#[cfg(unix)]
mod lifecycle;
mod manage;
#[cfg(unix)]
mod mcp;
mod rules;

/// Writes `contents` at `path`, making its directories.
pub(super) fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A Claude Code plugin `name` under `paths`: a Skill `search`, the MCP
/// servers `servers` (`.mcp.json`) and `hooks` (`hooks/hooks.json`).
pub(super) fn claude_plugin(
    paths: &TestPaths,
    name: &str,
    servers: serde_json::Value,
    hooks: Option<serde_json::Value>,
) -> PathBuf {
    let root = paths.app_data_dir.join("sources").join(name);
    write(
        &root.join(".claude-plugin/plugin.json"),
        &json!({ "name": name, "version": "1.0.0", "description": format!("The {name} plugin.") })
            .to_string(),
    );
    write(
        &root.join("skills/search/SKILL.md"),
        "---\nname: search\ndescription: Searches the demo records.\n---\nSearch the records with the plugin's tools.\n",
    );
    write(
        &root.join(".mcp.json"),
        &json!({ "mcpServers": servers }).to_string(),
    );
    if let Some(hooks) = hooks {
        write(
            &root.join("hooks/hooks.json"),
            &json!({ "hooks": hooks }).to_string(),
        );
    }
    root
}

fn plugins_root(paths: &TestPaths) -> PathBuf {
    paths.data_dir.join("agent-test").join("plugins")
}

/// Installs the package at `source` as `id` into the service's store.
pub(super) fn install(paths: &TestPaths, source: &Path, id: &str) -> PathBuf {
    let store = PackageStore::open(plugins_root(paths), ExecutorManager::default());
    store
        .install_directory(source, &PluginId::parse(id).unwrap())
        .unwrap()
        .root
}

#[cfg(unix)]
pub(super) fn set_enabled(config: &AgentConfig, id: &str, enabled: bool) {
    config
        .set_package_settings(
            &PluginId::parse(id).unwrap(),
            Some(&hanafubuki_agent_packages::PluginSettings {
                enabled: Some(enabled),
                ..Default::default()
            }),
        )
        .unwrap();
}

pub(super) fn plugin_skill_ids(service: &AgentApplicationService) -> Vec<String> {
    service
        .list_skills()
        .unwrap()
        .entries
        .into_iter()
        .filter(|entry| entry.id.plugin().is_some())
        .map(|entry| entry.id.as_str().to_string())
        .collect()
}

pub(super) async fn package(
    service: &AgentApplicationService,
    id: &str,
) -> crate::AgentPackageStatus {
    service
        .packages()
        .await
        .unwrap()
        .into_iter()
        .find(|package| package.id == id)
        .unwrap()
}

pub(super) async fn finish(service: AgentApplicationService, paths: TestPaths) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
