//! The `plugins` row: the model's view of the enabled installed plugins.
//! When a turn is admitted it freezes a summary of each enabled package (its
//! name, its description, its MCP servers that run) on the turn; every model
//! request of the turn then lists the packages that contribute Skills (in
//! the turn's Skill catalog) or MCP servers, and none when no package does.
//! The packages' content itself reaches the Agent through the `skills`,
//! `mcp` and `hooks` rows. The row also mounts the guest host
//! (`hanafubuki-agent-plugin-packages`), which runs the code packages'
//! guests as its child fibers, so turning this row off hides the summary
//! and stops every guest. While it runs, a change to a linked package's
//! directory, which the guest host watches, reloads the installed plugins
//! in the background: their content is read again, and that package's
//! guest starts anew.

mod prompt;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_contracts::{
    spawn_scoped_worker, CancellationToken, ContextRendererRegistry, PluginFaults,
    PromptContributorRegistry, Register, TurnAdmissionContributor, TurnAdmissionFuture,
    TurnAdmissionRegistry, TurnAdmissionRequest, TurnExtensions,
};
use kardia_core::{Context, Inject, PluginDef, Service};
use serde_json::Value;

use self::prompt::{AvailablePluginsPrompt, AVAILABLE_PLUGINS_RENDERER};
use crate::packages::{AgentPackages, PackageReload};

/// The row's name.
pub(crate) const PACKAGES_PLUGIN: &str = "plugins";

/// An enabled package as the turn's summary carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PluginSummary {
    /// `<plugin>@<marketplace>`.
    pub(crate) id: String,
    pub(crate) display_name: String,
    /// What its Skills' names are prefixed with.
    pub(crate) namespace: String,
    pub(crate) description: Option<String>,
    /// Its MCP servers that run.
    pub(crate) mcp_servers: Vec<String>,
}

/// The enabled packages of a turn, in key order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AvailablePlugins(pub(crate) Vec<PluginSummary>);

/// How often the linked packages' follower checks that the row stops.
const CANCELLATION_CHECK: Duration = Duration::from_millis(50);

pub(crate) fn packages_plugin(reload: PackageReload) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let reload = reload.clone();
        async move {
            let packages = reload.packages.clone();
            let guests = packages.guests().plugin();
            let changes = packages.guests().linked_changes();
            spawn_scoped_worker(&ctx, "plugins-linked", move |cancellation| {
                follow_linked(reload, changes, cancellation)
            })?;
            ctx.get::<TurnAdmissionRegistry>()?
                .register(Arc::new(PluginTurnAdmission(packages)))?;
            ctx.get::<PromptContributorRegistry>()?
                .register(Arc::new(AvailablePluginsPrompt))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(AVAILABLE_PLUGINS_RENDERER)?;
            ctx.plugin_def(&guests, Value::Null)?;
            Ok(())
        }
    })
    .name(PACKAGES_PLUGIN)
    .inject(
        Inject::new()
            .with(TurnAdmissionRegistry::NAME)
            .with(PromptContributorRegistry::NAME)
            .with(ContextRendererRegistry::NAME)
            .with(PluginFaults::NAME),
    )
}

/// Reloads the installed plugins in the background after each change to a
/// linked package's directory, until the row stops. Changes made while a
/// reload runs are taken by one more.
async fn follow_linked(
    reload: PackageReload,
    mut changes: tokio::sync::watch::Receiver<u64>,
    cancellation: CancellationToken,
) {
    loop {
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            () = cancelled(&cancellation) => return,
        }
        reload.spawn();
    }
}

async fn cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_CHECK).await;
    }
}

struct PluginTurnAdmission(AgentPackages);

impl TurnAdmissionContributor for PluginTurnAdmission {
    fn id(&self) -> &str {
        PACKAGES_PLUGIN
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a TurnAdmissionRequest,
        _cancellation: CancellationToken,
    ) -> TurnAdmissionFuture<'a> {
        let loaded = self.0.loaded();
        let summaries = loaded
            .enabled()
            .map(|package| PluginSummary {
                id: package.id.key(),
                display_name: package.display_name.clone(),
                namespace: package.namespace.clone(),
                description: package.description.clone(),
                mcp_servers: package
                    .mcp_servers
                    .iter()
                    .filter(|server| server.enabled)
                    .map(|server| server.id.as_str().to_string())
                    .collect(),
            })
            .collect();
        Box::pin(async move {
            Ok(TurnExtensions::default().with(Arc::new(AvailablePlugins(summaries))))
        })
    }
}
