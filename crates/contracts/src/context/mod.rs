//! The context contract: a plugin registers [`ContextContributor`]s in the
//! [`ContextRegistry`]; at turn admission they produce the sections of the
//! Agent's world state, which the kernel versions, diffs into history and
//! shows the model. How the model is shown a section, world state or prompt
//! fragment, is the [`ContextRenderer`] its owning plugin registers in the
//! [`ContextRendererRegistry`].

mod kind;
mod registry;
mod renderer;

use std::future::Future;
use std::pin::Pin;

use hanafubuki_agent_approval::AgentPermissionMode;
use hanafubuki_agent_foundation::cancellation::CancellationToken;

pub use kind::ContextSectionKind;
pub use registry::ContextRegistry;
pub use renderer::{
    ContextRenderer, ContextRendererRegistry, ContextRendererSnapshot, ContextRole, ContextSlot,
    WORLD_STATE_PREAMBLE,
};

use crate::{AgentSessionId, AgentSessionMode, TurnExtensions};

/// One section of the world state: model-visible text of one kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSection {
    pub kind: ContextSectionKind,
    pub content: String,
}

impl ContextSection {
    pub fn new(kind: ContextSectionKind, content: String) -> Self {
        Self { kind, content }
    }

    /// Renders a JSON payload with sorted object keys, so the model-visible
    /// text and its version do not depend on how the payload map was built
    /// (`serde_json/preserve_order` may be enabled anywhere in the build).
    pub fn from_json(kind: ContextSectionKind, payload: &serde_json::Value) -> Self {
        Self::new(
            kind,
            hanafubuki_agent_tool_contracts::canonical_json(payload).to_string(),
        )
    }
}

/// The turn a world state is collected for.
#[derive(Clone, Debug)]
pub struct ContextRequest {
    pub session_id: AgentSessionId,
    pub mode: AgentSessionMode,
    pub permission_mode: AgentPermissionMode,
    /// The text of the user's message the turn was submitted with; `None`
    /// for a turn that has none (implementing a plan, say).
    pub user_message: Option<String>,
    /// Host inputs keyed by type (for example the UI context the user
    /// submitted with the turn). A contributor reads the type it knows and
    /// treats a missing value as "not supplied for this turn".
    pub turn: TurnExtensions,
}

pub type ContextFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<ContextSection>, String>> + Send + 'a>>;

/// Produces sections of the world state for each admitted turn.
///
/// A contributor returns only kinds it declares in [`Self::kinds`], at most
/// one section per kind; an omitted kind is removed from the world state. An
/// error fails the admission of the turn with its text. Long work runs on the
/// host's blocking service and observes `cancellation`, which the registry
/// cancels when the contributor's plugin stops.
pub trait ContextContributor: Send + Sync + 'static {
    /// Unique within the registry.
    fn id(&self) -> &str;

    /// The kinds this contributor may produce. No two contributors declare the
    /// same kind.
    fn kinds(&self) -> &[ContextSectionKind];

    fn contribute<'a>(
        &'a self,
        request: &'a ContextRequest,
        cancellation: CancellationToken,
    ) -> ContextFuture<'a>;
}
