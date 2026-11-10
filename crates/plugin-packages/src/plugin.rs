//! The guest host's plugin, which the `plugins` row mounts: a kardia-loader
//! whose tree holds one entry per code package, and the supervisor that
//! applies the packages to it and follows its guests.

use std::sync::Arc;

use hanafubuki_agent_contracts::{spawn_scoped_worker, PluginFaults};
use kardia_core::{Context, Inject, PluginDef, Service, StatusChanged};
use kardia_hmr::{GuestExited, GuestStarted};
use kardia_loader::Loader;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::guests::{GuestHost, Mount, Shared};
use crate::settings::SettingsTree;
use crate::supervise;

/// The guest host plugin's name.
pub const GUESTS_PLUGIN: &str = "plugin-guests";
const SUPERVISOR_PLUGIN: &str = "plugin-guest-supervisor";

impl GuestHost {
    /// The guest host's plugin. Its guests are fibers in its loader's tree,
    /// which stop with it: turning the row that mounts it off stops every
    /// guest. The loader resolves each entry to a guest plugin and writes
    /// its own changes back through the settings port; the supervisor
    /// applies the packages once the loader runs.
    pub fn plugin(&self) -> PluginDef {
        let shared = self.0.clone();
        PluginDef::new(move |ctx: Context, _| {
            let shared = shared.clone();
            async move {
                let tree = Arc::new(SettingsTree::new(shared.settings.clone()));
                let loader = Loader::plugin_with(Arc::new(shared.resolver.clone()), tree);
                ctx.plugin_def(&loader, Value::Null)?;
                ctx.plugin_def(&supervisor(shared), Value::Null)?;
                Ok(())
            }
        })
        .name(GUESTS_PLUGIN)
    }
}

fn supervisor(shared: Arc<Shared>) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let shared = shared.clone();
        async move {
            let loader = (*ctx.get::<Loader>()?).clone();
            let (restarts, jobs) = mpsc::unbounded_channel();
            let listener = shared.clone();
            ctx.on(move |event: &StatusChanged| listener.on_status(&event.fiber, event.new))?;
            let listener = shared.clone();
            ctx.on(move |event: &GuestStarted| listener.on_started(&event.fiber, &event.handle))?;
            let listener = shared.clone();
            ctx.on(move |event: &GuestExited| listener.on_exited(&event.fiber, &event.exit))?;
            shared.mounted(Mount { loader, restarts });
            let worker = shared.clone();
            spawn_scoped_worker(&ctx, "plugin-guests", move |cancellation| {
                supervise::run(worker, jobs, cancellation)
            })?;
            // Registered after the worker, so it runs first when the plugin
            // stops: dropping the sender ends the worker at once.
            ctx.on_dispose(move || shared.unmounted())?;
            Ok(())
        }
    })
    .name(SUPERVISOR_PLUGIN)
    .inject(Inject::new().with(Loader::NAME).with(PluginFaults::NAME))
}
