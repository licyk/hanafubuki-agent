//! Turn admission: a plugin registers [`TurnAdmissionContributor`]s in the
//! [`TurnAdmissionRegistry`]; when a turn is admitted they produce the turn's
//! [`TurnExtensions`], which the kernel freezes on the turn and hands to its
//! tool calls and prompt contributors.

mod registry;

use std::future::Future;
use std::pin::Pin;

use hanafubuki_agent_foundation::cancellation::CancellationToken;

pub use registry::TurnAdmissionRegistry;

use crate::{AgentSessionId, AgentSessionMode, TurnExtensions};

/// The turn being admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnAdmissionRequest {
    pub session_id: AgentSessionId,
    /// The mode the turn runs in.
    pub mode: AgentSessionMode,
}

pub type TurnAdmissionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TurnExtensions, String>> + Send + 'a>>;

/// Produces per-turn data once, when a turn is admitted.
///
/// The values it returns are frozen for the turn: every tool call and every
/// sampling step of the turn reads the same ones. No two contributors return
/// a value of the same type. An error fails the admission of the turn with
/// its text. Long work runs on the host's blocking service and observes
/// `cancellation`, which the registry cancels when the contributor's plugin
/// stops.
pub trait TurnAdmissionContributor: Send + Sync + 'static {
    /// Unique within the registry.
    fn id(&self) -> &str;

    /// Contributors run in `(rank, id)` order, lowest rank first.
    fn rank(&self) -> u32;

    fn contribute<'a>(
        &'a self,
        request: &'a TurnAdmissionRequest,
        cancellation: CancellationToken,
    ) -> TurnAdmissionFuture<'a>;
}
