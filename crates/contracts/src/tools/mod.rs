//! The tool contract: a plugin registers [`AgentToolHandler`]s in the
//! [`ToolRegistry`], or an [`AgentToolSource`] that plans handlers per
//! sampling step in the [`ToolSourceRegistry`]; the kernel plans, gates and
//! runs them. An [`AgentToolPresenter`] in the [`ToolPresenterRegistry`]
//! says how an interface shows their calls.

mod approval;
pub mod arguments;
mod display;
mod error;
mod extensions;
mod handler;
mod lease;
mod presenter;
mod registry;
mod source;

pub use approval::verify_tool_approval;
pub use display::{
    output_text, structured_output, AgentListingEntry, AgentListingEntryKind, AgentMutationKind,
    AgentOutputBlock, AgentPatchFile, AgentPatchHunkStart, AgentPatchLine, AgentPatchLineKind,
    AgentPatchOperation, AgentPatchPreview, AgentSearchMatch, AgentStyleSpan, AgentStyledText,
    AgentTextColor, AgentTextStyle, AgentToolDisplay, AgentToolDisplayMetadata, AgentWebResult,
};
pub use error::{decode_arguments, object_arguments, AgentToolError, ToolCallValidationError};
pub use extensions::TurnExtensions;
pub use handler::{
    AgentMemoryContextEffect, AgentToolHandler, ToolAvailability, ToolCallContext, ToolCallOrigin,
    ToolContextFragment, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolPlanContext,
    ToolResult,
};
pub use lease::ToolPlanLease;
pub use presenter::{
    AgentToolPresenter, ArgumentSubjects, ToolPresenterRegistry, MAX_TOOL_SUBJECT_CHARS,
};
pub use registry::{ToolRegistry, ToolSourceRegistry, RESERVED_TOOL_NAMES};
pub use source::{
    AgentToolOmission, AgentToolSource, SourcedTool, ToolSourceContribution, ToolSourceStep,
    ToolSourceTrust, MAX_AGENT_TOOL_OMISSIONS,
};
