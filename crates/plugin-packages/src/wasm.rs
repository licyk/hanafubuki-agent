//! WASM guests: a package's module, run by kardia-hmr's wasmtime backend
//! inside the Agent's process, one store per start. A module can reach
//! nothing but the guest ABI's host functions (`view` and `invoke` limited
//! as for any guest), so it needs no process, sandbox or grant. Its limits
//! bound each export: fuel and time (by epoch interruption), and the
//! linear memory it may grow to.
//!
//! The module is compiled, and its metadata read from one instance of it,
//! when its plugin is first resolved; the metadata must be the manifest's
//! name providing only [`GUEST_SERVICE`], as a process guest's. A guest
//! that traps, runs out of fuel or time, or outgrows its memory fails the
//! call; [`GuestHost::call`](crate::GuestHost::call) then fails its fiber,
//! so it restarts as a process guest that exits does.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use hanafubuki_agent_packages::{PackageContributes, WasmRuntime};
use kardia_core::{Error, PluginDef};
use kardia_hmr::{Exports, ModuleCode, ModuleGraph, ModuleSource, WasmLimits, WasmSource};

use crate::package::{GuestPackage, GUEST_SERVICE};
use crate::resolver::guest_name;
use crate::spawn::{check_content, in_package, resolve};

/// What a WASM call's error says when the instance itself went wrong (a
/// trap, its fuel or time used up, a memory it may not grow), not the guest
/// reporting an error of its own. kardia-hmr hands both over as text.
const INSTANCE_FAILURES: &[&str] = &[
    "wasm trap:",
    "forcing trap when growing",
    "the guest was stopped",
    "error while executing at wasm backtrace",
];

/// The plugin of `package`'s WASM guest, named `guest:<id>`; one whose
/// module cannot run fails its start with the reason. Blocking: it hashes
/// the package and compiles the module.
pub(crate) fn plugin(
    package: &GuestPackage,
    runtime: &WasmRuntime,
    call_timeout: Duration,
) -> PluginDef {
    let started = Instant::now();
    log::debug!("plugin {}: its module loads", package.id);
    let loaded = load(package, runtime, call_timeout);
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

/// Whether `error`, from a call into a WASM guest, means its instance
/// failed: it is not called again, but started anew.
pub(crate) fn instance_failed(error: &str) -> bool {
    INSTANCE_FAILURES.iter().any(|mark| error.contains(mark))
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
) -> Result<PluginDef, String> {
    check_content(package)?;
    let root = resolve(&package.root, "its package directory")?;
    let module = in_package(&root, &runtime.module)?;
    let mut limits = WasmLimits::new()
        .timeout(time_limit(runtime, &package.contributes, call_timeout))
        .memory(usize::try_from(runtime.limits.memory_bytes).unwrap_or(usize::MAX));
    if let Some(fuel) = runtime.limits.fuel {
        limits = limits.fuel(fuel);
    }
    let graph = ModuleGraph::new(OneModule(WasmSource::with_limits(limits)));
    let exports = graph
        .import(module.native_text())
        .map_err(|error| format!("its module did not load: {error}"))?;
    let Exports::Plugin(plugin) = exports else {
        return Err("its module exports no plugin".to_string());
    };
    check_meta(&plugin, package)?;
    Ok(plugin)
}

/// The guest's own metadata is what the manifest makes it: its name,
/// injecting nothing, providing [`GUEST_SERVICE`] only.
fn check_meta(plugin: &PluginDef, package: &GuestPackage) -> Result<(), String> {
    let provides: BTreeSet<&str> = plugin
        .provide_spec()
        .iter()
        .map(|key| key.as_str())
        .collect();
    let injects: Vec<&str> = plugin
        .inject_spec()
        .keys()
        .map(|key| key.as_str())
        .collect();
    if plugin.plugin_name() == Some(package.name.as_str())
        && injects.is_empty()
        && provides == BTreeSet::from([GUEST_SERVICE])
    {
        return Ok(());
    }
    Err(format!(
        "the guest declares the plugin {:?} injecting {injects:?} and providing {provides:?}, \
         but its manifest makes it {:?}, providing only `{GUEST_SERVICE}`",
        plugin.plugin_name().unwrap_or_default(),
        package.name,
    ))
}

/// The package's module alone: a guest imports no other module.
struct OneModule(WasmSource);

impl ModuleSource for OneModule {
    fn load(&self, id: &str) -> kardia_core::Result<ModuleCode> {
        let code = self.0.load(id)?;
        if !code.deps.is_empty() {
            return Err(Error::msg(
                "it declares other modules it depends on (`deps`), which a guest cannot have",
            ));
        }
        Ok(code)
    }
}
