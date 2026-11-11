//! The installed plugins' content: the packages in the store that the
//! settings leave on, each read for its Skill roots, its MCP servers, its
//! hooks and its roles, which the Agent's Skill catalog, MCP side, hooks
//! and role catalog then follow.
//! One package that cannot be read, or one of its components, costs only
//! itself: what went wrong is that package's issue.
//!
//! The set is loaded when the Agent is built and again by every reload
//! (`AgentApplicationService::reload_packages`), which the facade runs after
//! a settings change and after each change to the installed packages, and
//! the `plugins` row after a change to a linked package's directory, which
//! the guest host watches. A package in this project's format with a
//! runtime also has code, which the guest host runs
//! (`hanafubuki-agent-plugin-packages`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent_config::{AgentConfig, AgentSettings, RoleDefinition};
use hanafubuki_agent_hooks::{HooksEngine, PluginHookSource};
use hanafubuki_agent_mcp::McpServerId;
use hanafubuki_agent_packages::{ManifestFormat, PackagePermissions, PackageStore, PluginId};
use hanafubuki_agent_plugin_packages::{GuestHost, GuestPackage, LinkedPackage};
use hanafubuki_agent_skills::PluginSkillRoot;
use parking_lot::RwLock;

use crate::roles::AgentPackageRoles;
use crate::service::{AgentMcpApplication, McpPluginServer};
use crate::skills::AgentSkillApplication;

mod content;
mod guest;
mod lifecycle;
mod marketplaces;
mod reload;
mod roles;
mod status;
mod validate;

pub(crate) use guest::guest_host;
pub use lifecycle::{
    AgentMarketplace, AgentMarketplaceAdded, AgentMarketplacePlugin, AgentMarketplaceUpgrade,
    AgentPackageChange, AgentPackageChangeKind, AgentPackageInstall, AgentPackageSource,
    AgentPackageUninstall, LOCAL_MARKETPLACE,
};
pub(crate) use marketplaces::{configured_marketplace, marketplace_view};
pub(crate) use reload::PackageReload;
pub use status::{AgentPackageContent, AgentPackageGuest, AgentPackageStatus};
pub use validate::{validate_package, AgentPackageValidation};

/// The installed packages and the content of those that are on, as last
/// loaded, with the sources that follow them; shared with the `plugins`
/// row, which summarizes them for the model.
#[derive(Clone)]
pub(crate) struct AgentPackages {
    store: Arc<PackageStore>,
    /// The Agent runs in safe mode: the packages are listed, and none
    /// contributes anything.
    safe_mode: bool,
    loaded: Arc<RwLock<Arc<LoadedPackages>>>,
    /// Held by a reload from its read of the settings to its last change,
    /// so the later of two reloads is the one that stays.
    reload: Arc<tokio::sync::Mutex<()>>,
    skills: AgentSkillApplication,
    mcp: AgentMcpApplication,
    hooks: Arc<HooksEngine>,
    /// The code packages' guests, which the `plugins` row runs.
    guests: GuestHost,
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
    /// Its roles, named `<plugin>:<role>`; none when another package of its
    /// plugin name comes first.
    pub(crate) roles: Vec<RoleDefinition>,
    /// Its code, on or off.
    pub(crate) guest: Option<GuestPackage>,
    /// The directory it is linked to, while it is linked and active.
    pub(crate) linked: Option<PathBuf>,
    /// What its code's settings are (`configSchema`).
    pub(crate) config_schema: Option<serde_json::Map<String, serde_json::Value>>,
    /// The options its `configSchema` lowers to and the settings' values.
    pub(crate) options: Option<crate::host::AgentPluginOptionsStatus>,
    pub(crate) issues: Vec<String>,
}

impl AgentPackages {
    /// The packages of `store`, loaded as `config` leaves them on, with the
    /// Skill catalog, the hooks and the MCP side following them; the MCP
    /// servers connect with the Agent's start, and the guests start with
    /// the `plugins` row. Blocking.
    pub(crate) fn open(
        store: PackageStore,
        safe_mode: bool,
        config: &AgentConfig,
        skills: AgentSkillApplication,
        mcp: AgentMcpApplication,
        hooks: Arc<HooksEngine>,
        guests: impl FnOnce(Vec<GuestPackage>) -> GuestHost,
    ) -> Result<Self, String> {
        let settings = config.settings()?;
        let installed = Self::read(&store, safe_mode, &settings, &user_servers(config)?);
        let packages = Self {
            store: Arc::new(store),
            safe_mode,
            loaded: Arc::default(),
            reload: Arc::default(),
            skills,
            mcp,
            hooks,
            guests: guests(installed.guest_packages()),
        };
        packages.guests.follow_linked(installed.linked());
        *packages.mcp.plugin_servers.write() = installed.mcp_servers();
        packages.follow(installed);
        Ok(packages)
    }

    /// The store every change to the installed packages goes through: one
    /// per data directory, since opening one sweeps its staging area.
    pub(crate) fn store(&self) -> &Arc<PackageStore> {
        &self.store
    }

    pub(crate) fn guests(&self) -> &GuestHost {
        &self.guests
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
        let guests = loaded.guest_packages();
        self.guests.follow_linked(loaded.linked());
        self.follow(loaded);
        self.mcp.set_plugin_servers(servers).await;
        self.guests.apply(guests).await;
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

    /// Every installed package, with what it contributes and its hooks'
    /// trust (each package with hooks hashed). Blocking: reads the Skill
    /// catalog, the hooks and the install record.
    pub(crate) fn status(&self) -> Vec<AgentPackageStatus> {
        let skills = self
            .skills
            .list()
            .map(|catalog| catalog.entries)
            .unwrap_or_default();
        let hooks = self.hooks.list().hooks;
        let installed = self.store.installed();
        self.loaded()
            .packages
            .iter()
            .map(|package| {
                let record = installed
                    .iter()
                    .find(|installed| installed.id == package.id)
                    .and_then(|installed| installed.record.as_ref());
                let guest = self.guest_status(package);
                status::package_status(package, record, &skills, &hooks, guest)
            })
            .collect()
    }

    /// The hooks of the package `id` that wait to be trusted. Blocking:
    /// hashes the packages with hooks.
    pub(crate) fn hooks_to_trust(&self, id: &PluginId) -> Vec<hanafubuki_agent_hooks::HookEntry> {
        let key = id.key();
        self.hooks
            .list()
            .hooks
            .into_iter()
            .filter(|hook| {
                matches!(&hook.source, hanafubuki_agent_hooks::HookSource::Plugin { id, .. } if *id == key)
                    && hook.trust != hanafubuki_agent_hooks::HookTrust::Trusted
            })
            .collect()
    }

    fn guest_status(&self, package: &LoadedPackage) -> Option<AgentPackageGuest> {
        let guest = package.guest.as_ref()?;
        Some(AgentPackageGuest {
            runtime: guest.runtime.kind(),
            permissions: guest.permissions.clone(),
            granted_permissions: guest.granted.clone(),
            status: self.guests.status(&guest.id.key())?,
            settings: guest.settings.clone(),
            config_schema: package.config_schema.clone(),
            options: package.options.clone(),
        })
    }

    /// Records that the user granted the code of `id` `permissions`.
    /// Blocking; the guest follows at the next reload.
    pub(crate) fn grant(
        &self,
        id: &PluginId,
        permissions: PackagePermissions,
    ) -> Result<(), String> {
        self.store
            .grant_permissions(id, permissions)
            .map_err(|error| error.to_string())
    }

    /// Reads the installed packages and the content of those `settings`
    /// leave on (none in safe mode); `user_servers` are the user's MCP servers, which a
    /// package's server of the same name gives way to. Blocking.
    pub(crate) fn load(
        &self,
        settings: &AgentSettings,
        user_servers: &BTreeSet<McpServerId>,
    ) -> LoadedPackages {
        Self::read(&self.store, self.safe_mode, settings, user_servers)
    }

    fn read(
        store: &PackageStore,
        safe_mode: bool,
        settings: &AgentSettings,
        user_servers: &BTreeSet<McpServerId>,
    ) -> LoadedPackages {
        let mut installed = store.installed();
        installed.sort_by_key(|package| package.id.key());
        let mut packages = installed
            .iter()
            .map(|package| {
                let entry = settings
                    .plugins
                    .packages
                    .get(&package.id.key())
                    .cloned()
                    .unwrap_or_default();
                let enabled = settings.package_enabled(&package.id);
                content::load(package, enabled, enabled && !safe_mode, &entry)
            })
            .collect::<Vec<_>>();
        resolve_server_names(&mut packages, user_servers);
        resolve_role_names(&mut packages);
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

    /// The roles of each package that is on.
    pub(crate) fn roles(&self) -> Vec<AgentPackageRoles> {
        self.enabled()
            .filter(|package| !package.roles.is_empty())
            .map(|package| AgentPackageRoles {
                id: package.id.key(),
                definitions: package.roles.clone(),
            })
            .collect()
    }

    /// The linked packages that are active, which the guest host watches.
    pub(crate) fn linked(&self) -> Vec<LinkedPackage> {
        self.enabled()
            .filter_map(|package| {
                let root = package.linked.clone()?;
                Some(LinkedPackage {
                    id: package.id.clone(),
                    root,
                })
            })
            .collect()
    }

    /// The code packages, on or off.
    pub(crate) fn guest_packages(&self) -> Vec<GuestPackage> {
        self.packages
            .iter()
            .filter_map(|package| package.guest.clone())
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

/// Keeps the roles of a plugin name (`<plugin>:…`) to one package: the
/// first by id with roles; another package of that plugin name, from
/// another marketplace, contributes none, and that is its issue.
fn resolve_role_names(packages: &mut [LoadedPackage]) {
    let mut owners = BTreeMap::<String, String>::new();
    for package in packages
        .iter_mut()
        .filter(|package| package.enabled && !package.roles.is_empty())
    {
        let plugin = &package.id.plugin_name;
        match owners.get(plugin) {
            Some(owner) => {
                package.issues.push(format!(
                    "its roles are left out: `{owner}` comes first, and its roles are named \
                     `{plugin}:…` already"
                ));
                package.roles.clear();
            }
            None => {
                owners.insert(plugin.clone(), package.id.key());
            }
        }
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
