use hanafubuki_agent_contracts::tools::arguments::{clamped_usize, optional_text, ArgumentNotes};
use hanafubuki_agent_contracts::{object_arguments, ToolCallValidationError, ToolResult};
use hanafubuki_agent_model_api::ModelToolCall;

use crate::tool_catalog::MAX_AGENT_TOOL_LIST_RESULTS;
use crate::{AgentError, AgentService, AgentToolOutput, AgentToolPlan};

impl AgentService {
    pub(super) fn execute_plan_list(
        &self,
        call: &ModelToolCall,
        plan: &AgentToolPlan,
    ) -> Result<AgentToolOutput, AgentError> {
        let arguments = match ListArguments::parse(&call.arguments) {
            Ok(arguments) => arguments,
            Err(error) => {
                return Ok(AgentToolOutput::model_visible_error(
                    serde_json::json!({"error": error.to_string()}),
                ))
            }
        };
        let page = plan.list_deferred(
            arguments.source.as_deref(),
            arguments.offset,
            arguments.limit,
        );
        let output =
            serde_json::to_value(page).map_err(|error| AgentError::Protocol(error.to_string()))?;
        Ok(AgentToolOutput::typed(
            ToolResult::success(output)
                .with_argument_notes(&arguments.notes)
                .output,
        ))
    }
}

struct ListArguments {
    source: Option<String>,
    offset: usize,
    limit: usize,
    notes: ArgumentNotes,
}

impl ListArguments {
    fn parse(arguments: &serde_json::Value) -> Result<Self, ToolCallValidationError> {
        let object = object_arguments(arguments)?;
        let mut notes = ArgumentNotes::default();
        let source = optional_text(object, "source", usize::MAX)?;
        let offset = clamped_usize(object, "offset", 0, usize::MAX, 0, &mut notes)?;
        let limit = clamped_usize(
            object,
            "limit",
            1,
            MAX_AGENT_TOOL_LIST_RESULTS,
            MAX_AGENT_TOOL_LIST_RESULTS,
            &mut notes,
        )?;
        Ok(Self {
            source,
            offset,
            limit,
            notes,
        })
    }
}
