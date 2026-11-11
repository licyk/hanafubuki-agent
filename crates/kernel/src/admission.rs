//! Collects a turn's extensions from the turn admission contributors plugins
//! registered.

use std::time::Duration;

use hanafubuki_agent_contracts::{
    ExtensionPoint, TurnAdmissionRegistry, TurnAdmissionRequest, TurnExtensions,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::bounded_call::{bounded, overran, Bounded};
use crate::{AgentCapabilityScope, AgentError};

/// Runs every contributor of one registry snapshot, one after another in
/// `(rank, id)` order, but those of a plugin the session's `scope` turns
/// off, and merges what they return into the turn's extensions.
///
/// A contributor that fails (an error, a panic reported as a fault of its
/// plugin, no answer within `deadline`), whose plugin is stopping, or that returns a value of a type
/// another contributor already returned is left out of this turn: its
/// values are absent, as if its plugin had not started, so its tools see a
/// missing value, and its plugin shows the issue in its status until a later
/// admission succeeds. An explicit Skill reference the turn then cannot
/// resolve still fails the turn (the kernel's own check).
///
/// Each call observes a child of `admission`: cancelling it cancels the call
/// in flight and fails the collection with `Cancelled`, and so does dropping
/// the collection.
pub async fn collect_turn_extensions(
    registry: &TurnAdmissionRegistry,
    request: &TurnAdmissionRequest,
    scope: &AgentCapabilityScope,
    admission: &CancellationToken,
    deadline: Duration,
) -> Result<TurnExtensions, AgentError> {
    let mut extensions = TurnExtensions::default();
    for entry in registry.ordered() {
        // Its values are absent, as for a stopped plugin's.
        if scope.turns_off(entry.owner()) {
            continue;
        }
        let contributor = entry.item();
        let call = admission.child();
        let Some(_lease) = entry.acquire(&call) else {
            log::debug!(
                "agent turn admission contributor {} left out: its plugin is stopping",
                contributor.id()
            );
            continue;
        };
        let contributed = entry.guard_async(ExtensionPoint::TurnAdmission, |contributor| {
            contributor.contribute(request, call.clone())
        });
        let issue = match bounded(contributed, call.clone(), admission, deadline).await {
            Bounded::Returned(Ok(Ok(contributed))) => (!extensions.merge_disjoint(contributed))
                .then(|| "it produced a value another contributor produced".to_string()),
            Bounded::Returned(Ok(Err(error))) => Some(error),
            Bounded::Returned(Err(_)) => Some("it failed with an internal error".to_string()),
            Bounded::Overran => Some(overran(deadline)),
            Bounded::Cancelled => return Err(AgentError::Cancelled),
        };
        entry.faults().set_issue(
            entry.owner(),
            &format!("{} {}", ExtensionPoint::TurnAdmission, contributor.id()),
            issue,
        );
    }
    Ok(extensions)
}

#[cfg(test)]
mod tests;
