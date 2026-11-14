use std::future::Future;
use std::pin::Pin;

use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::{
    FreeformToolSpec, ToolExecutionClass, ToolExposure, ToolName, ToolOutput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::AgentToolError;
use crate::{
    AgentActivityAction, AgentGeneration, AgentRevision, AgentSessionId, AgentSessionMode,
    AgentTurnId, PreparedAgentImage,
};

pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Whether a tool changes anything outside the conversation. Only read-only
/// tools are available in Plan mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    ReadOnly,
    Mutating,
}

/// Whether a tool's output brings untrusted external content into the
/// conversation, which stops the session from contributing to memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMemoryContextEffect {
    #[default]
    None,
    PollutesContribution,
}

/// Everything the kernel needs to plan, gate and present a tool, fixed for
/// the lifetime of its registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDescriptor {
    pub name: ToolName,
    pub description: String,
    pub input_schema: Value,
    pub exposure: ToolExposure,
    pub execution_class: ToolExecutionClass,
    pub effect: ToolEffect,
    pub action: AgentActivityAction,
    pub memory_effect: AgentMemoryContextEffect,
    /// How long one `invoke` may run before the kernel cancels it and answers
    /// the model with a timeout error; `None` takes the turn's tool timeout
    /// (`AgentTurnBudgetPolicy::tool_idle_timeout`). Neither the permission
    /// check nor the wait for an approval counts.
    pub timeout: Option<std::time::Duration>,
}

/// The call a tool runs in. Identities come from the kernel; nothing here is
/// model-provided except through `ToolInvocation::arguments`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallContext {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub mode: AgentSessionMode,
    pub session_revision: AgentRevision,
    pub call_id: String,
    /// Whether the session's model accepts image input right now.
    pub image_input: bool,
    pub origin: ToolCallOrigin,
}

/// Who made a call. The kernel sets it; no argument can.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolCallOrigin {
    /// The model asked for it.
    #[default]
    Model,
    /// The person typed it (a `!command`), which approves it: the kernel
    /// asks no one further unless an approval rule requires a person.
    User,
}

/// What a sampling step's model and provider can take, for tools whose
/// availability or spec depends on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolPlanContext {
    pub image_input: bool,
    pub multimodal_tool_output: bool,
    pub freeform_tools: bool,
}

/// How a tool appears in one sampling step's plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolAvailability {
    /// Offered as a function tool with the descriptor's schema.
    Function,
    /// Offered as a freeform tool; the model sends raw text, which arrives as
    /// a JSON string argument.
    Freeform(FreeformToolSpec),
    Omitted,
}

pub struct ToolInvocation {
    pub context: ToolCallContext,
    /// The model's raw arguments. The handler validates them; its
    /// `ToolCallValidationError` texts are what the model reads back.
    pub arguments: Value,
    /// Cancelled with the turn, and when the tool's plugin stops while the
    /// call outlives the drain grace period.
    pub cancellation: CancellationToken,
    /// Live progress for the activity monitor (command output, status).
    pub progress: std::sync::Arc<dyn crate::AgentToolProgressSink>,
    /// The leases the step's tool sources froze into its plan.
    pub plan_leases: Vec<std::sync::Arc<dyn super::ToolPlanLease>>,
    /// The turn's typed data, which the kernel fills (the frozen Skill
    /// catalog).
    pub turn: super::TurnExtensions,
}

impl ToolInvocation {
    /// The plan lease of type `T`, which the tool's own source returned.
    pub fn plan_lease<T: std::any::Any>(&self) -> Option<&T> {
        self.plan_leases
            .iter()
            .find_map(|lease| lease.as_ref().as_any().downcast_ref::<T>())
    }
}

/// Instructions a tool adds to the conversation right after its output.
///
/// The kernel commits a fragment once per turn and source, as the context of
/// the Skill `source` names (a Skill id; anything else is a tool error), and
/// the model reads it as developer context from then on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolContextFragment {
    pub source: String,
    pub name: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResult {
    pub output: ToolOutput,
    /// Images the kernel stages into history after the output.
    pub images: Vec<PreparedAgentImage>,
    /// Context the kernel commits after the output.
    pub context: Option<ToolContextFragment>,
    /// What an interface's display of the output adds (a command's
    /// colours); committed beside the output, never sent to the model.
    pub display_metadata: super::AgentToolDisplayMetadata,
}

impl ToolResult {
    pub fn success(value: Value) -> Self {
        ToolOutput::structured(value, false).into()
    }

    /// A result the model sees as a tool error without the call failing.
    pub fn model_visible_error(value: Value) -> Self {
        ToolOutput::structured(value, true).into()
    }

    pub fn image(image: PreparedAgentImage) -> Self {
        Self {
            output: ToolOutput {
                content: Vec::new(),
                structured_content: None,
                is_error: false,
            },
            images: vec![image],
            context: None,
            display_metadata: Default::default(),
        }
    }

    pub fn with_context(mut self, context: ToolContextFragment) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_display_metadata(mut self, metadata: super::AgentToolDisplayMetadata) -> Self {
        self.display_metadata = metadata;
        self
    }

    /// Appends how the handler adjusted the model's arguments (a clamped
    /// limit) as a text item after the output, so the model reads what was
    /// actually used.
    pub fn with_argument_notes(mut self, notes: &super::arguments::ArgumentNotes) -> Self {
        if let Some(text) = notes.render() {
            self.output
                .content
                .push(hanafubuki_agent_tool_contracts::ToolOutputContent::Text { text });
        }
        self
    }
}

impl From<Value> for ToolResult {
    fn from(value: Value) -> Self {
        Self::success(value)
    }
}

impl From<ToolOutput> for ToolResult {
    fn from(output: ToolOutput) -> Self {
        Self {
            output,
            images: Vec::new(),
            context: None,
            display_metadata: Default::default(),
        }
    }
}

/// A tool contributed by a plugin through [`super::ToolRegistry`].
///
/// The kernel runs every handler through the same pipeline: mode policy,
/// [`AgentToolHandler::permission`], the approval policy when review is
/// required, then [`AgentToolHandler::invoke`] with the one-shot approval.
/// `invoke` must re-check that the approval still matches the call it runs.
pub trait AgentToolHandler: Send + Sync + 'static {
    fn descriptor(&self) -> &ToolDescriptor;

    fn availability(&self, _plan: &ToolPlanContext) -> ToolAvailability {
        ToolAvailability::Function
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>>;

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>>;
}
