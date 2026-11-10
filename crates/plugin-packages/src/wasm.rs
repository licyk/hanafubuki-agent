//! WASM guests: a package's module, run by kardia-hmr's wasmtime backend
//! inside the Agent's process, one store per start. A module can reach
//! nothing but the guest ABI's host functions (`view` and `invoke` limited
//! as for any guest), so it needs no process, sandbox or grant. Its limits
//! bound each export: fuel and time (by epoch interruption), and the
//! linear memory it may grow to.
//!
//! The module is compiled when its plugin is first resolved, and nothing
//! is instantiated then: its metadata comes from the manifest, the
//! manifest's name providing only [`GUEST_SERVICE`], as a process guest's,
//! and each instance must declare the same when it starts. Guests with the
//! same limits share one engine ([`WasmEngines`]), and so one epoch thread.
//! A guest that traps, runs out of fuel or time, or outgrows its memory
//! fails the call with a typed failure; [`GuestHost::call`](crate::GuestHost::call)
//! then fails its fiber, so it restarts as a process guest that exits does.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_packages::{PackageContributes, PackageRuntime, WasmRuntime};
use kardia_core::{Error, PluginDef};
use kardia_hmr::{set_macos_mach_ports, GuestMeta, WasmLimits, WasmSource};
use parking_lot::Mutex;

use crate::package::{GuestPackage, GUEST_SERVICE};
use crate::resolver::guest_name;
use crate::spawn::{check_content, in_package, resolve};

/// The engines of the WASM guests, one per set of limits, kept while a
/// package needs it. Clones share them.
#[derive(Clone, Default)]
pub(crate) struct WasmEngines(Arc<Mutex<HashMap<Limits, Arc<WasmSource>>>>);

/// A module's limits, as its engine is made with them.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Limits {
    fuel: Option<u64>,
    timeout: Duration,
    memory: usize,
}

impl WasmEngines {
    /// Drops the engines no package of `packages` needs. A running guest
    /// keeps its own engine alive until it stops.
    pub(crate) fn follow(&self, packages: &[GuestPackage], call_timeout: Duration) {
        let needed: HashSet<Limits> = packages
            .iter()
            .filter_map(|package| match &package.runtime {
                PackageRuntime::Wasm(runtime) => Some(Limits::of(package, runtime, call_timeout)),
                PackageRuntime::Process(_) => None,
            })
            .collect();
        self.0.lock().retain(|limits, _| needed.contains(limits));
    }

    /// The engine for `limits`, made on first need.
    fn source(&self, limits: Limits) -> Arc<WasmSource> {
        self.0
            .lock()
            .entry(limits)
            .or_insert_with(|| {
                use_signals_on_macos();
                Arc::new(WasmSource::with_limits(limits.wasm()))
            })
            .clone()
    }
}

impl Limits {
    fn of(package: &GuestPackage, runtime: &WasmRuntime, call_timeout: Duration) -> Self {
        Self {
            fuel: runtime.limits.fuel,
            timeout: time_limit(runtime, &package.contributes, call_timeout),
            memory: usize::try_from(runtime.limits.memory_bytes).unwrap_or(usize::MAX),
        }
    }

    fn wasm(self) -> WasmLimits {
        let limits = WasmLimits::new().timeout(self.timeout).memory(self.memory);
        match self.fuel {
            Some(fuel) => limits.fuel(fuel),
            None => limits,
        }
    }
}

/// wasmtime handles traps on macOS through Mach exception ports by
/// default, a process-wide mechanism of its own beside the trap signals
/// every platform uses: the Agent asks for signals, as on Linux, before its
/// first engine. A host that made an engine with Mach ports first keeps
/// them, which every later engine then uses.
fn use_signals_on_macos() {
    if let Err(error) = set_macos_mach_ports(false) {
        log::warn!("WASM guests keep wasmtime's Mach exception ports: {error}");
    }
}

/// The plugin of `package`'s WASM guest, named `guest:<id>`; one whose
/// module cannot be compiled fails its start with the reason. Blocking: it
/// hashes the package and compiles the module.
pub(crate) fn plugin(
    package: &GuestPackage,
    runtime: &WasmRuntime,
    call_timeout: Duration,
    engines: &WasmEngines,
) -> PluginDef {
    let started = Instant::now();
    log::debug!("plugin {}: its module loads", package.id);
    let loaded = load(package, runtime, call_timeout, engines);
    let outcome = if loaded.is_ok() {
        "loaded"
    } else {
        "did not load"
    };
    log::debug!(
        "plugin {}: its module {outcome} in {} ms",
        package.id,
        started.elapsed().as_millis()
    );
    match loaded {
        Ok(plugin) => plugin.name(guest_name(package)),
        Err(error) => failing(package, error),
    }
}

/// A plugin of `package`'s guest whose every start fails with `error`.
pub(crate) fn failing(package: &GuestPackage, error: String) -> PluginDef {
    PluginDef::new(move |_, _| {
        let error = error.clone();
        async move { Err(Error::msg(error)) }
    })
    .name(guest_name(package))
}

/// How long each export may run: the manifest's `timeoutMs`, else the
/// longest any call of the guest may take, the call deadline or a longer
/// tool `timeout`. Each call is stopped at its own deadline too.
fn time_limit(
    runtime: &WasmRuntime,
    contributes: &PackageContributes,
    call_timeout: Duration,
) -> Duration {
    runtime.limits.timeout.unwrap_or_else(|| {
        contributes
            .tools
            .iter()
            .filter_map(|tool| tool.timeout)
            .fold(call_timeout, Duration::max)
    })
}

fn load(
    package: &GuestPackage,
    runtime: &WasmRuntime,
    call_timeout: Duration,
    engines: &WasmEngines,
) -> Result<PluginDef, String> {
    check_content(package)?;
    let root = resolve(&package.root, "its package directory")?;
    let module = in_package(&root, &runtime.module)?;
    let bytes = std::fs::read(module.system())
        .map_err(|error| format!("its module did not read: {error}"))?;
    let source = engines.source(Limits::of(package, runtime, call_timeout));
    let guest = source
        .guest(bytes)
        .map_err(|error| format!("its module did not load: {error}"))?;
    Ok(guest.plugin(GuestMeta::new(&package.name).provide(GUEST_SERVICE)))
}
