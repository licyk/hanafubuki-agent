//! What each plugin contributes now, read from the kernel registries.
//!
//! Every entry records the plugin that registered it (`RegistryEntry::owner`,
//! the plugin's name), so one snapshot per registry answers for all plugins.
//! Snapshots copy `Arc`s under each registry's own lock; nothing here runs a
//! plugin's code beyond its identity accessors.

use std::collections::HashMap;

use hanafubuki_agent_contracts::{RegistryService, RegistrySnapshot};
use hanafubuki_agent_kernel::AgentKernelRegistries;

use super::status::{AgentPluginContextContribution, AgentPluginContributions};

/// The contributions of every plugin that has an entry, by plugin name.
pub(super) fn contributions_by_plugin(
    registries: &AgentKernelRegistries,
) -> HashMap<String, AgentPluginContributions> {
    let mut by_plugin = HashMap::<String, AgentPluginContributions>::new();
    collect(
        &mut by_plugin,
        registries.tools.registry().snapshot(),
        |into, tool| into.tools.push(tool.descriptor().name.qualified()),
    );
    collect(
        &mut by_plugin,
        registries.tool_sources.registry().snapshot(),
        |into, source| into.tool_sources.push(source.id().to_string()),
    );
    collect(
        &mut by_plugin,
        registries.context.registry().snapshot(),
        |into, contributor| {
            into.context.push(AgentPluginContextContribution {
                id: contributor.id().to_string(),
                kinds: contributor
                    .kinds()
                    .iter()
                    .map(|kind| kind.as_str().to_string())
                    .collect(),
            })
        },
    );
    collect(
        &mut by_plugin,
        registries.turn_admission.registry().snapshot(),
        |into, contributor| into.turn_admission.push(contributor.id().to_string()),
    );
    collect(
        &mut by_plugin,
        registries.prompt.registry().snapshot(),
        |into, contributor| into.prompt.push(contributor.id().to_string()),
    );
    collect(
        &mut by_plugin,
        registries.approval_rules.registry().snapshot(),
        |into, rule| into.approval_rules.push(rule.id().to_string()),
    );
    collect(
        &mut by_plugin,
        registries.observers.registry().snapshot(),
        |into, observer| into.observers.push(observer.id().to_string()),
    );
    by_plugin
}

fn collect<T>(
    by_plugin: &mut HashMap<String, AgentPluginContributions>,
    snapshot: RegistrySnapshot<T>,
    mut add: impl FnMut(&mut AgentPluginContributions, &T),
) {
    for entry in snapshot.entries() {
        add(
            by_plugin.entry(entry.owner().to_string()).or_default(),
            entry.item(),
        );
    }
}
