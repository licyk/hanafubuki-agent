//! The `plugins` row: the model's view of the enabled installed plugins.
//! When a turn is admitted it freezes a summary of each enabled package (its
//! name, its description, its MCP servers that run) on the turn; every model
//! request of the turn then lists the packages that contribute Skills (in
//! the turn's Skill catalog) or MCP servers, and none when no package does.
//! The packages' content itself reaches the Agent through the `skills`,
//! `mcp` and `hooks` rows, so turning this row off hides only the summary.

mod prompt;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    CancellationToken, ContextRendererRegistry, PromptContributorRegistry, Register,
    TurnAdmissionContributor, TurnAdmissionFuture, TurnAdmissionRegistry, TurnAdmissionRequest,
    TurnExtensions,
};
use kardia_core::{Context, Inject, PluginDef, Service};

use self::prompt::{AvailablePluginsPrompt, AVAILABLE_PLUGINS_RENDERER};
use crate::packages::AgentPackages;

/// The row's name. The guest host takes it over when installed plugins can
/// carry code (docs/plan-plugin-system.md, §4.2).
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

pub(crate) fn packages_plugin(packages: AgentPackages) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let packages = packages.clone();
        async move {
            ctx.get::<TurnAdmissionRegistry>()?
                .register(Arc::new(PluginTurnAdmission(packages)))?;
            ctx.get::<PromptContributorRegistry>()?
                .register(Arc::new(AvailablePluginsPrompt))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(AVAILABLE_PLUGINS_RENDERER)?;
            Ok(())
        }
    })
    .name(PACKAGES_PLUGIN)
    .inject(
        Inject::new()
            .with(TurnAdmissionRegistry::NAME)
            .with(PromptContributorRegistry::NAME)
            .with(ContextRendererRegistry::NAME),
    )
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
