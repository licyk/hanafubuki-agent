//! The loader's resolver: a guest entry's name to its plugin, a kardia-hmr
//! process guest whose metadata comes from the manifest, so resolving it
//! starts nothing, or a WASM guest, whose module is compiled (and its
//! metadata read) once, on the blocking pool, at its first resolution; and
//! its host-services entry's name to the plugin that provides what the
//! guest may view. Each name keeps one plugin, the stable identity
//! kardia-loader needs across restarts; a new version, install or grant is
//! a new name.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use hanafubuki_agent_host::GUEST_LOGGER_PREFIX;
use hanafubuki_agent_packages::{PackageRuntime, ProcessRuntime, WasmRuntime};
use kardia_core::{Error, PluginDef};
use kardia_hmr::{GuestCommand, GuestMeta, ProcessGuest};
use kardia_loader::{ModuleNotFound, Resolver};
use parking_lot::RwLock;
use tokio::sync::OnceCell;

use crate::host_services::host_services;
use crate::package::{GuestPackage, GUEST_SERVICE};
use crate::spawn::{GuestSpawner, SpawnContext};
use crate::wasm;

/// The longest line a guest may send.
const FRAME_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct GuestResolver {
    plugins: Arc<RwLock<HashMap<String, Resolved>>>,
}

/// An entry name's plugin, or what makes it at the first resolution.
#[derive(Clone)]
enum Resolved {
    Plugin(PluginDef),
    Wasm(Arc<WasmLoad>),
}

/// A WASM guest's plugin, made once: compiling its module may take a
/// while, so it is not made while the packages are applied.
struct WasmLoad {
    package: GuestPackage,
    runtime: WasmRuntime,
    call_timeout: Duration,
    plugin: OnceCell<PluginDef>,
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
                .or_insert_with(|| Resolved::Plugin(host_services(package, context)));
        }
    }
}

impl Resolver for GuestResolver {
    fn resolve(&self, name: &str) -> BoxFuture<'static, Result<PluginDef, Error>> {
        let resolved = self.plugins.read().get(name).cloned();
        let Some(resolved) = resolved else {
            let missing = ModuleNotFound {
                name: name.to_string(),
            };
            return std::future::ready(Err(missing.into())).boxed();
        };
        async move {
            match resolved {
                Resolved::Plugin(plugin) => Ok(plugin),
                Resolved::Wasm(load) => Ok(load.plugin().await),
            }
        }
        .boxed()
    }
}

impl WasmLoad {
    async fn plugin(&self) -> PluginDef {
        self.plugin
            .get_or_init(|| async {
                let (package, runtime) = (self.package.clone(), self.runtime.clone());
                let call_timeout = self.call_timeout;
                let made = tokio::task::spawn_blocking(move || {
                    wasm::plugin(&package, &runtime, call_timeout)
                });
                made.await.unwrap_or_else(|error| {
                    wasm::failing(&self.package, format!("its module was not loaded: {error}"))
                })
            })
            .await
            .clone()
    }
}

fn guest_plugin(package: &GuestPackage, context: &Arc<SpawnContext>) -> Resolved {
    match &package.runtime {
        PackageRuntime::Process(runtime) => {
            Resolved::Plugin(process_plugin(package, runtime, context))
        }
        PackageRuntime::Wasm(runtime) => Resolved::Wasm(Arc::new(WasmLoad {
            package: package.clone(),
            runtime: runtime.clone(),
            call_timeout: context.call_timeout,
            plugin: OnceCell::new(),
        })),
    }
}

/// The package's process guest: its process spawned by [`GuestSpawner`],
/// every export bounded by the call timeout, the plugin named
/// `guest:<id>` (so the Agent's log exporter treats what it logs as the
/// guest's content), declaring the manifest's name and [`GUEST_SERVICE`].
fn process_plugin(
    package: &GuestPackage,
    runtime: &ProcessRuntime,
    context: &Arc<SpawnContext>,
) -> PluginDef {
    let program = runtime.command.first().cloned().unwrap_or_default();
    ProcessGuest::new(GuestCommand::new(program))
        .spawner(GuestSpawner::new(
            package.clone(),
            runtime.clone(),
            context.clone(),
        ))
        .timeout(context.call_timeout)
        .handshake_timeout(context.call_timeout)
        .frame_limit(FRAME_LIMIT)
        .plugin(GuestMeta::new(&package.name).provide(GUEST_SERVICE))
        .name(guest_name(package))
}

/// `guest:<id>`: the Agent's log exporter treats what a plugin of this
/// name logs as the guest's content.
pub(crate) fn guest_name(package: &GuestPackage) -> String {
    format!("{GUEST_LOGGER_PREFIX}{}", package.id.key())
}
