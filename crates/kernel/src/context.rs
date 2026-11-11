//! Collects a turn's world state from the context contributors plugins
//! registered.

use std::collections::BTreeMap;
use std::time::Duration;

use hanafubuki_agent_contracts::{
    ContextRegistry, ContextRequest, ContextSectionKind, ExtensionPoint, RegistryService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::bounded_call::{bounded, overran, Bounded};
use crate::{AgentCapabilityScope, AgentError, AgentWorldStateSection, AgentWorldStateSnapshot};

/// Runs every contributor of one registry snapshot, one after another,
/// but those of a plugin the session's `scope` turns off, and returns
/// their sections in kind name order, so the history records them
/// the same way every time; the model is shown them in their renderers'
/// slot order.
///
/// A contributor that fails (an error, a panic reported as a fault of its
/// plugin, no answer within `deadline`) or whose plugin is stopping is left
/// out of this turn: its kinds are absent, as if its plugin had not started,
/// and its plugin shows the issue in its status until a later turn's contribution succeeds. A section
/// of a kind its contributor did not declare, or of a kind already produced,
/// is left out the same way, and so is a plugin kind past
/// `MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS`. The sections' sizes are
/// validated by the admission itself.
///
/// Each call observes a child of `admission`: cancelling it cancels the call
/// in flight and fails the collection with `Cancelled`, and so does dropping
/// the collection.
pub async fn collect_world_state(
    registry: &ContextRegistry,
    request: &ContextRequest,
    scope: &AgentCapabilityScope,
    admission: &CancellationToken,
    deadline: Duration,
) -> Result<AgentWorldStateSnapshot, AgentError> {
    let snapshot = registry.registry().snapshot();
    let mut sections = BTreeMap::new();
    for entry in snapshot.entries() {
        // Its kinds are absent, as for a stopped plugin's.
        if scope.turns_off(entry.owner()) {
            continue;
        }
        let contributor = entry.item();
        let call = admission.child();
        let Some(_lease) = entry.acquire(&call) else {
            log::debug!(
                "agent context contributor {} left out: its plugin is stopping",
                contributor.id()
            );
            continue;
        };
        let contributed = entry.guard_async(ExtensionPoint::Context, |contributor| {
            contributor.contribute(request, call.clone())
        });
        let answer = match bounded(contributed, call.clone(), admission, deadline).await {
            Bounded::Returned(Ok(Ok(contributed))) => Ok(contributed),
            Bounded::Returned(Ok(Err(error))) => Err(error),
            Bounded::Returned(Err(_)) => Err("it failed with an internal error".to_string()),
            Bounded::Overran => Err(overran(deadline)),
            Bounded::Cancelled => return Err(AgentError::Cancelled),
        };
        let contributed = match answer {
            Ok(contributed) => contributed,
            Err(issue) => {
                leave_out(entry.faults(), entry.owner(), contributor.id(), issue);
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
