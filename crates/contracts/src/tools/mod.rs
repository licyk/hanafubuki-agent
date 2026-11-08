//! The tool contract: a plugin registers [`AgentToolHandler`]s in the
//! [`ToolRegistry`], or an [`AgentToolSource`] that plans handlers per
//! sampling step in the [`ToolSourceRegistry`]; the kernel plans, gates and
//! runs them.

mod approval;
pub mod arguments;
mod error;
mod extensions;
mod handler;
mod lease;
mod registry;
mod source;

pub use approval::verify_tool_approval;
pub use error::{decode_arguments, object_arguments, AgentToolError, ToolCallValidationError};
pub use extensions::TurnExtensions;
pub use handler::{
    AgentMemoryContextEffect, AgentToolHandler, ToolAvailability, ToolCallContext,
    ToolContextFragment, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolPlanContext,
    ToolResult,
};
pub use lease::ToolPlanLease;
pub use registry::{ToolRegistry, ToolSourceRegistry, RESERVED_TOOL_NAMES};
pub use source::{
    AgentToolOmission, AgentToolSource, SourcedTool, ToolSourceContribution, ToolSourceStep,
    ToolSourceTrust, MAX_AGENT_TOOL_OMISSIONS,
};
