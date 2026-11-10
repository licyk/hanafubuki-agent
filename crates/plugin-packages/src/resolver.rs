//! The loader's resolver: a guest entry's name to its plugin, a kardia-hmr
//! process guest whose metadata comes from the manifest, so resolving it
//! starts nothing; and its host-services entry's name to the plugin that
//! provides what the guest may view. Each name keeps one plugin, the stable identity
//! kardia-loader needs across restarts; a new version, install or grant is
//! a new name.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use hanafubuki_agent_host::GUEST_LOGGER_PREFIX;
use kardia_core::{Error, PluginDef};
use kardia_hmr::{GuestCommand, GuestMeta, ProcessGuest};
use kardia_loader::{ModuleNotFound, Resolver};
use parking_lot::RwLock;

use crate::host_services::host_services;
use crate::package::{GuestPackage, GUEST_SERVICE};
use crate::spawn::{GuestSpawner, SpawnContext};

/// The longest line a guest may send.
const FRAME_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct GuestResolver {
    plugins: Arc<RwLock<HashMap<String, PluginDef>>>,
}

impl GuestResolver {
    /// Keeps the plugins of `packages`, building those it does not have,
    /// and drops the rest.
    pub(crate) fn follow(&self, packages: &[GuestPackage], context: &Arc<SpawnContext>) {
        let names: HashSet<String> = packages
            .iter()
            .flat_map(|package| [package.entry_name(), package.host_entry_name()])
            .collect();
        let mut plugins = self.plugins.write();
        plugins.retain(|name, _| names.contains(name));
        for package in packages {
            plugins
                .entry(package.entry_name())
                .or_insert_with(|| guest_plugin(package, context));
            plugins
                .entry(package.host_entry_name())
                .or_insert_with(|| host_services(package, context));
        }
    }
}

impl Resolver for GuestResolver {
    fn resolve(&self, name: &str) -> BoxFuture<'static, Result<PluginDef, Error>> {
        let plugin = self.plugins.read().get(name).cloned().ok_or_else(|| {
            ModuleNotFound {
                name: name.to_string(),
            }
            .into()
        });
        std::future::ready(plugin).boxed()
    }
}

/// The package's guest: its process spawned by [`GuestSpawner`], every
/// export bounded by the call timeout, the plugin named
/// `guest:<id>` (so the Agent's log exporter treats what it logs as the
/// guest's content), declaring the manifest's name and [`GUEST_SERVICE`].
fn guest_plugin(package: &GuestPackage, context: &Arc<SpawnContext>) -> PluginDef {
    let program = package.runtime.command.first().cloned().unwrap_or_default();
    ProcessGuest::new(GuestCommand::new(program))
        .spawner(GuestSpawner::new(package.clone(), context.clone()))
        .timeout(context.call_timeout)
        .handshake_timeout(context.call_timeout)
        .frame_limit(FRAME_LIMIT)
        .plugin(GuestMeta::new(&package.name).provide(GUEST_SERVICE))
        .name(format!("{GUEST_LOGGER_PREFIX}{}", package.id.key()))
}
