//! Calls the kernel answers with a model-visible error instead of running a
//! tool: a name the step does not offer, and arguments that could not be
//! used. Each still gets its output item, so the call/output pair stays valid
//! for every provider protocol and the turn goes on.

use crate::tool_catalog::{AgentPlannedTool, AgentToolPlan};
use crate::tool_runtime::AgentToolOutput;
use crate::{AgentToolGeneration, AgentToolId, AgentToolPresentation, AgentToolProviderId};
use hanafubuki_agent_contracts::AgentActivityAction;
use hanafubuki_agent_model_api::{ModelToolArgumentError, ModelToolCall};
use serde_json::json;

/// What the kernel does with one call of a sampling step.
pub(in crate::service) enum CallDisposition {
    /// The step offers the tool and its arguments arrived intact.
    Run(AgentPlannedTool),
    /// The call is answered with `output`, a model-visible error, and the
    /// tool never runs. `registration` is the planned tool it names, when
    /// the step has one:
    /// - a Deferred tool the step did not admit (discover it first);
    /// - an offered tool whose arguments could not be used;
    /// - none: the step offers no tool of this name. Unknown, Hidden,
    ///   mode-hidden and provider-incompatible tools all get one identical
    ///   error, so a hidden tool stays indistinguishable from an absent one.
    Reject {
        registration: Option<AgentPlannedTool>,
        output: AgentToolOutput,
    },
}

impl CallDisposition {
    pub(in crate::service) fn of(plan: &AgentToolPlan, call: &ModelToolCall) -> Self {
        let registration = match plan.resolve_call(call) {
            Ok(registration) => registration.clone(),
            Err(_) => {
                return match plan.unselected_deferred_call(call) {
                    Some(registration) => Self::Reject {
                        registration: Some(registration.clone()),
                        output: undiscovered_tool_output(&call.name),
                    },
                    None => Self::Reject {
                        registration: None,
                        output: unavailable_tool_output(&call.name),
                    },
                };
            }
        };
        match &call.argument_error {
            Some(error) => Self::Reject {
                registration: Some(registration),
                output: invalid_arguments_output(&call.name, error),
            },
            None => Self::Run(registration),
        }
    }

    /// The name the shell shows for the call: the planned tool's own name,
    /// or the wire name for a name the step does not offer.
    pub(in crate::service) fn tool_name<'a>(&'a self, call: &'a ModelToolCall) -> &'a str {
        match self {
            Self::Run(registration)
            | Self::Reject {
                registration: Some(registration),
                ..
            } => &registration.model_spec.name,
            Self::Reject {
                registration: None, ..
            } => &call.name,
        }
    }

    /// The presentation the shell shows for the call; a generic tool for a
    /// name the step does not offer.
    pub(in crate::service) fn presentation(&self) -> AgentToolPresentation {
        match self {
            Self::Run(registration)
            | Self::Reject {
                registration: Some(registration),
                ..
            } => registration.presentation.clone(),
            Self::Reject {
                registration: None, ..
            } => AgentToolPresentation {
                action: AgentActivityAction::USE_TOOL,
                subject: None,
            },
        }
    }
}

fn undiscovered_tool_output(name: &str) -> AgentToolOutput {
    AgentToolOutput::model_visible_error(json!({
        "error": "tool requires discovery before use",
        "tool": name,
        "recovery": {
            "tool": "tool_search",
            "query": name,
            "retry": "after the tool search result"
        }
    }))
}

fn invalid_arguments_output(name: &str, error: &ModelToolArgumentError) -> AgentToolOutput {
    AgentToolOutput::model_visible_error(json!({
        "error": format!("the tool arguments could not be used: {}", error.reason),
        "tool": name,
        "receivedArguments": error.preview,
        "retry": "call the tool again with one JSON object that matches its schema"
    }))
}

/// The one error every call of a name the step does not offer receives.
pub(in crate::service) fn unavailable_tool_output(name: &str) -> AgentToolOutput {
    AgentToolOutput::model_visible_error(json!({
        "error": format!("no tool named `{name}` is available in this step")
    }))
}

/// The identity a history `ToolCall` item records for a name the step did
/// not offer. It names no provider or tool, so nothing resolves it.
pub(in crate::service) fn unavailable_tool_identity(
) -> (AgentToolProviderId, AgentToolId, AgentToolGeneration) {
    (
        AgentToolProviderId::parse("unavailable").expect("static identifier is valid"),
        AgentToolId::parse("unavailable").expect("static identifier is valid"),
        AgentToolGeneration([0; 32]),
    )
}
