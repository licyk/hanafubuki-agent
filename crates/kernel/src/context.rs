//! Collects a turn's world state from the context contributors plugins
//! registered.

use std::collections::BTreeMap;

use hanafubuki_agent_contracts::{
    ContextRegistry, ContextRequest, ContextSectionKind, ExtensionPoint, RegistryService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::{AgentError, AgentWorldStateSection, AgentWorldStateSnapshot};

/// Runs every contributor of one registry snapshot, one after another, and
/// returns their sections in kind name order, so the history records them
/// the same way every time; the model is shown them in their renderers'
/// slot order.
///
/// A contributor that fails (an error, a panic reported as a fault of its
/// plugin) or whose plugin is stopping is left out of this turn: its kinds
/// are absent, as if its plugin had not started, and its plugin shows the
/// issue in its status until a later turn's contribution succeeds. A section
/// of a kind its contributor did not declare, or of a kind already produced,
/// is left out the same way, and so is a plugin kind past
/// `MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS`. The sections' sizes are
/// validated by the admission itself.
pub async fn collect_world_state(
    registry: &ContextRegistry,
    request: &ContextRequest,
) -> Result<AgentWorldStateSnapshot, AgentError> {
    let snapshot = registry.registry().snapshot();
    let mut sections = BTreeMap::new();
    for entry in snapshot.entries() {
        let contributor = entry.item();
        let cancellation = CancellationToken::default();
        let Some(_lease) = entry.acquire(&cancellation) else {
            log::debug!(
                "agent context contributor {} left out: its plugin is stopping",
                contributor.id()
            );
            continue;
        };
        let contributed = match entry
            .guard_async(ExtensionPoint::Context, |contributor| {
                contributor.contribute(request, cancellation.clone())
            })
            .await
        {
            Ok(Ok(contributed)) => contributed,
            Ok(Err(error)) => {
                leave_out(entry.faults(), entry.owner(), contributor.id(), error);
                continue;
            }
            Err(_) => {
                leave_out(
                    entry.faults(),
                    entry.owner(),
                    contributor.id(),
                    "it failed with an internal error".to_string(),
                );
                continue;
            }
        };
        let mut issue = None;
        for section in contributed {
            if !contributor.kinds().contains(&section.kind) {
                issue = Some(format!("it produced the undeclared kind {}", section.kind));
                continue;
            }
            let kind = section.kind.clone();
            if sections.contains_key(&kind) {
                issue = Some(format!(
                    "the context section kind {kind} was produced twice"
                ));
                continue;
            }
            // Plugin kinds share a bound the built-in kinds do not use up;
            // one past it is left out, not the turn.
            if !ContextSectionKind::KNOWN.contains(&kind)
                && sections
                    .keys()
                    .filter(|kind| !ContextSectionKind::KNOWN.contains(kind))
                    .count()
                    >= crate::MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS
            {
                issue = Some(format!(
                    "the context section kind {kind} is past the bound of {} plugin kinds",
                    crate::MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS
                ));
                continue;
            }
            sections.insert(kind, AgentWorldStateSection::from(section));
        }
        match issue {
            Some(issue) => leave_out(entry.faults(), entry.owner(), contributor.id(), issue),
            None => entry
                .faults()
                .set_issue(entry.owner(), &source(contributor.id()), None),
        }
    }
    Ok(AgentWorldStateSnapshot {
        sections: sections.into_values().collect(),
    })
}

fn leave_out(
    faults: &hanafubuki_agent_contracts::PluginFaults,
    plugin: &str,
    contributor: &str,
    issue: String,
) {
    faults.set_issue(plugin, &source(contributor), Some(issue));
}

fn source(contributor: &str) -> String {
    format!("{} {contributor}", ExtensionPoint::Context)
}

#[cfg(test)]
mod tests;
