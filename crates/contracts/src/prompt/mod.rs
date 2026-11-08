//! Prompt fragments: a plugin registers [`PromptContributor`]s in the
//! [`PromptContributorRegistry`]; for every model request of a turn they
//! produce model-visible sections from the turn's [`TurnExtensions`] and from
//! what they prepared for the request's sampling step.
//!
//! A fragment is shown by its kind's [`ContextRenderer`](crate::ContextRenderer).
//! The kernel places the fragments with the sections the conversation opens
//! with, ordered by slot, in every request: after a compaction they are
//! rendered again, not summarized. Instructions and world state are not
//! prompt fragments.

mod registry;

use std::future::Future;
use std::pin::Pin;

use hanafubuki_agent_foundation::cancellation::CancellationToken;

pub use registry::{PromptContributorRegistry, PromptContributorSnapshot};

use crate::{AgentGeneration, AgentSessionId, AgentTurnId, ContextSectionKind, TurnExtensions};

/// One model-visible fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptFragment {
    pub kind: ContextSectionKind,
    /// The content version, shown to the model only by a versioned renderer.
    pub version: Option<String>,
    pub content: String,
}

/// The sampling step a contributor prepares for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptStep {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    /// The turn's sampling step, counted from 1.
    pub sampling_step: u32,
}

/// What a contributor renders from.
#[derive(Clone, Copy, Debug)]
pub struct PromptRequest<'a> {
    /// The turn's extensions, frozen at its admission.
    pub turn: &'a TurnExtensions,
    /// What the contributors prepared for the sampling step this request
    /// belongs to. `None` for a request built outside a sampling step (the
    /// compaction estimates), which no contributor prepared for.
    pub prepared: Option<&'a TurnExtensions>,
    /// The session's configured context window, when it has one.
    pub context_window_tokens: Option<u64>,
}

pub type PromptPrepareFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TurnExtensions, String>> + Send + 'a>>;

/// Renders fragments for every model request of a turn.
///
/// `prepare` loads, once per sampling step, what the step's fragments depend
/// on beyond the turn (the memory summary as it stands). The kernel calls it
/// when the step starts, with no lock held, under a lease on the
/// contributor's entry and with a child of the turn's cancellation, which the
/// registry also cancels when the contributor's plugin stops. It may do
/// bounded I/O; long work runs on the host's blocking service and observes
/// the cancellation. The values it returns, keyed by type, are handed to every
/// contributor's `contribute` for that step's request, and the kernel reads
/// the ones it knows (`AgentMemoryStep`). An error leaves the contributor's
/// values out of the step; the default prepares nothing.
///
/// `contribute` is a pure, bounded rendering of the request: it does no I/O,
/// never blocks, keeps no state per call and never calls the kernel. The
/// kernel calls it with no lock held, from the turn's extensions it read
/// under its lock, and builds the request from the result afterwards. It
/// returns no fragment for a request that lacks what it renders.
pub trait PromptContributor: Send + Sync + 'static {
    /// Unique within the registry.
    fn id(&self) -> &str;

    /// Steps are prepared in `(rank, id)` order, lowest rank first.
    fn rank(&self) -> u32;

    fn prepare<'a>(
        &'a self,
        _step: &'a PromptStep,
        _cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        Box::pin(async { Ok(TurnExtensions::default()) })
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment>;
}
