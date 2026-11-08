//! What a host submits beside a message. The Agent passes it on without
//! reading it: the host's own ports and plugins read the types they put in
//! (`TurnExtensions` is keyed by type), so a turn started without host input
//! (plan implementation, tests) simply carries none.

use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_kernel::AgentHostContext;

use super::DesktopAgentImageInput;

/// A user message that starts a turn.
pub struct AgentTurnRequest {
    pub text: String,
    pub images: Vec<DesktopAgentImageInput>,
    pub skills: Vec<hanafubuki_agent_kernel::AgentSkillRef>,
    /// Handed to the context contributors in the turn's `ContextRequest`.
    pub host: TurnExtensions,
}

/// A message that steers a running turn.
pub struct AgentSteeringRequest {
    pub text: String,
    pub images: Vec<DesktopAgentImageInput>,
    /// Rendered into the steering's host context ([`HostContextPort`]).
    pub host: TurnExtensions,
}

/// The host context a steering message carries, which the kernel writes to
/// the world state as a section of the context's kind when it changed.
pub trait HostContextPort: Send + Sync {
    /// The context for what the host submitted with the steering; `None` to
    /// carry none. Runs on the Agent's blocking pool and may block.
    fn steering_context(&self, host: &TurnExtensions) -> Option<AgentHostContext>;
}
