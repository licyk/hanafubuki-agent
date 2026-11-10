//! The installed plugins' content: the packages in the store that the
//! settings leave on, each read for its Skill roots, its MCP servers and its
//! hooks, which the Agent's Skill catalog, MCP side and hooks then follow.
//! One package that cannot be read, or one of its components, costs only
//! itself: what went wrong is that package's issue.
//!
//! The set is loaded when the Agent is built and again by every reload
//! (`AgentApplicationService::reload_packages`), which the facade runs after
//! a settings change and after each change to the installed packages.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use hanafubuki_agent_config::{AgentConfig, AgentSettings};
use hanafubuki_agent_hooks::{HooksEngine, PluginHookSource};
use hanafubuki_agent_mcp::McpServerId;
use hanafubuki_agent_packages::{ManifestFormat, PackageStore, PluginId};
use hanafubuki_agent_skills::PluginSkillRoot;
use parking_lot::RwLock;

use crate::service::{AgentMcpApplication, McpPluginServer};
use crate::skills::AgentSkillApplication;

mod content;
mod status;

pub use status::{AgentPackageContent, AgentPackageStatus};

/// The installed packages and the content of those that are on, as last
/// loaded, with the sources that follow them; shared with the `plugins`
/// row, which summarizes them for the model.
#[derive(Clone)]
pub(crate) struct AgentPackages {
    store: Arc<PackageStore>,
    loaded: Arc<RwLock<Arc<LoadedPackages>>>,
    /// Held by a reload from its read of the settings to its last change,
    /// so the later of two reloads is the one that stays.
    reload: Arc<tokio::sync::Mutex<()>>,
    skills: AgentSkillApplication,
    mcp: AgentMcpApplication,
    hooks: Arc<HooksEngine>,
}

/// Every installed package, by id, and what each that is on contributes.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoadedPackages {
    pub(crate) packages: Vec<LoadedPackage>,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedPackage {
    pub(crate) id: PluginId,
    pub(crate) version: String,
    pub(crate) format: Option<ManifestFormat>,
    pub(crate) enabled: bool,
    /// The name a person reads, and the one its Skills are prefixed with.
    pub(crate) display_name: String,
    pub(crate) namespace: String,
    pub(crate) description: Option<String>,
    pub(crate) skill_roots: Vec<PluginSkillRoot>,
    /// Its MCP servers that run: those no earlier source has a server of
    /// the same name for.
    pub(crate) mcp_servers: Vec<hanafubuki_agent_mcp::McpServerConfig>,
    pub(crate) hooks: Option<PluginHookSource>,
    pub(crate) issues: Vec<String>,
}

impl AgentPackages {
    /// The packages of `store`, loaded as `config` leaves them on, with the
    /// Skill catalog, the hooks and the MCP side following them; the MCP
    /// servers connect with the Agent's start. Blocking.
    pub(crate) fn open(
        store: PackageStore,
        config: &AgentConfig,
        skills: AgentSkillApplication,
        mcp: AgentMcpApplication,
        hooks: Arc<HooksEngine>,
    ) -> Result<Self, String> {
        let packages = Self {
            store: Arc::new(store),
            loaded: Arc::default(),
            reload: Arc::default(),
            skills,
            mcp,
            hooks,
        };
        let loaded = packages.load(&config.settings()?, &user_servers(config)?);
        *packages.mcp.plugin_servers.write() = loaded.mcp_servers();
        packages.follow(loaded);
        Ok(packages)
    }

    pub(crate) fn loaded(&self) -> Arc<LoadedPackages> {
        self.loaded.read().clone()
    }

    /// Loads the packages again as `config` leaves them on, and brings the
    /// Skill catalog, the hooks and the MCP servers up to date: a server
    /// added or changed has connected (or failed) when this returns.
    pub(crate) async fn reload(
        &self,
        config: AgentConfig,
        blocking: &dyn hanafubuki_agent_foundation::blocking::BlockingOperationPort,
    ) -> Result<(), String> {
        let _reload = self.reload.lock().await;
        let packages = self.clone();
        let loaded = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            blocking,
            "agent plugin packages load",
            move || Ok::<_, String>(packages.load(&config.settings()?, &user_servers(&config)?)),
        )
        .await??;
        let servers = loaded.mcp_servers();
        self.follow(loaded);
        self.mcp.set_plugin_servers(servers).await;
        Ok(())
    }

    /// The Skill roots and the hook sources of `loaded`, which is the set
    /// from now on. A Skill catalog that did not open is the `skills`
    /// row's failure, not the packages'.
    fn follow(&self, loaded: LoadedPackages) {
        self.hooks.set_plugin_sources(loaded.hook_sources());
        if let Err(error) = self.skills.set_plugin_roots(loaded.skill_roots()) {
            log::warn!("the installed plugins' Skills are not read: {error}");
        }
        *self.loaded.write() = Arc::new(loaded);
    }

    /// Every installed package, with what it contributes. Blocking: reads
    /// the Skill catalog and the hooks.
    pub(crate) fn status(&self) -> Vec<AgentPackageStatus> {
        let skills = self
            .skills
            .list()
            .map(|catalog| catalog.entries)
            .unwrap_or_default();
        let hooks = self.hooks.discover().hooks;
        self.loaded()
            .packages
            .iter()
            .map(|package| status::package_status(package, &skills, &hooks))
            .collect()
    }

    /// Reads the installed packages and the content of those `settings`
    /// leave on; `user_servers` are the user's MCP servers, which a
    /// package's server of the same name gives way to. Blocking.
    pub(crate) fn load(
        &self,
        settings: &AgentSettings,
        user_servers: &BTreeSet<McpServerId>,
    ) -> LoadedPackages {
        let mut installed = self.store.installed();
        installed.sort_by_key(|package| package.id.key());
        let mut packages = installed
            .iter()
            .map(|package| {
                let overlay = settings
                    .plugins
                    .packages
                    .get(&package.id.key())
                    .map(|settings| settings.mcp_servers.clone())
                    .unwrap_or_default();
                content::load(package, settings.package_enabled(&package.id), &overlay)
            })
            .collect::<Vec<_>>();
        resolve_server_names(&mut packages, user_servers);
        LoadedPackages { packages }
    }
}

impl LoadedPackages {
    pub(crate) fn enabled(&self) -> impl Iterator<Item = &LoadedPackage> {
        self.packages.iter().filter(|package| package.enabled)
    }

    pub(crate) fn skill_roots(&self) -> Vec<PluginSkillRoot> {
        self.enabled()
            .flat_map(|package| package.skill_roots.iter().cloned())
            .collect()
    }

    pub(crate) fn hook_sources(&self) -> Vec<PluginHookSource> {
        self.enabled()
            .filter_map(|package| package.hooks.clone())
            .collect()
    }

    pub(crate) fn mcp_servers(&self) -> Vec<McpPluginServer> {
        self.enabled()
            .flat_map(|package| {
                package.mcp_servers.iter().map(|server| McpPluginServer {
                    plugin: package.id.key(),
                    server: server.clone(),
                })
            })
            .collect()
    }
}

/// Keeps each MCP server name to one source: the user's server of a name
/// comes first, then the first package in key order; a package's server
/// that gives way is that package's issue.
fn resolve_server_names(packages: &mut [LoadedPackage], user_servers: &BTreeSet<McpServerId>) {
    let mut owners = BTreeMap::<McpServerId, String>::new();
    for package in packages.iter_mut().filter(|package| package.enabled) {
        let key = package.id.key();
        let mut issues = Vec::new();
        package.mcp_servers.retain(|server| {
            let name = server.id.as_str();
            if user_servers.contains(&server.id) {
                issues.push(format!(
                    "MCP server `{name}` is also configured in mcp.toml, which comes first"
                ));
                return false;
            }
            if let Some(owner) = owners.get(&server.id) {
                issues.push(format!(
                    "MCP server `{name}` is also declared by `{owner}`, which comes first"
                ));
                return false;
            }
            owners.insert(server.id.clone(), key.clone());
            true
        });
        package.issues.extend(issues);
    }
}

/// The names of the user's MCP servers in `mcp.toml`.
fn user_servers(config: &AgentConfig) -> Result<BTreeSet<McpServerId>, String> {
    let settings = config.mcp().load().map_err(|error| error.to_string())?;
    Ok(settings
        .servers
        .into_iter()
        .map(|server| server.id)
        .collect())
}
