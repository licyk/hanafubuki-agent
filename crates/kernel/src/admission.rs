//! Collects a turn's extensions from the turn admission contributors plugins
//! registered.

use hanafubuki_agent_contracts::{
    ExtensionPoint, TurnAdmissionRegistry, TurnAdmissionRequest, TurnExtensions,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::AgentError;

/// Runs every contributor of one registry snapshot, one after another in
/// `(rank, id)` order, and merges what they return into the turn's
/// extensions.
///
/// A contributor that fails (an error, a panic reported as a fault of its
/// plugin), whose plugin is stopping, or that returns a value of a type
/// another contributor already returned is left out of this turn: its
/// values are absent, as if its plugin had not started, so its tools see a
/// missing value, and its plugin shows the issue in its status until a later
/// admission succeeds. An explicit Skill reference the turn then cannot
/// resolve still fails the turn (the kernel's own check).
pub async fn collect_turn_extensions(
    registry: &TurnAdmissionRegistry,
    request: &TurnAdmissionRequest,
) -> Result<TurnExtensions, AgentError> {
    let mut extensions = TurnExtensions::default();
    for entry in registry.ordered() {
        let contributor = entry.item();
        let cancellation = CancellationToken::default();
        let Some(_lease) = entry.acquire(&cancellation) else {
            log::debug!(
                "agent turn admission contributor {} left out: its plugin is stopping",
                contributor.id()
            );
            continue;
        };
        let issue = match entry
            .guard_async(ExtensionPoint::TurnAdmission, |contributor| {
                contributor.contribute(request, cancellation.clone())
            })
            .await
        {
            Ok(Ok(contributed)) => (!extensions.merge_disjoint(contributed))
                .then(|| "it produced a value another contributor produced".to_string()),
            Ok(Err(error)) => Some(error),
            Err(_) => Some("it failed with an internal error".to_string()),
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
