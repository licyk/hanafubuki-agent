use crate::model::AgentError;
use crate::service::AgentService;
use crate::tool_runtime::AgentToolOutput;
use hanafubuki_agent_model_api::ModelToolCall;
use serde_json::json;

impl AgentService {
    pub(super) fn execute_plan_search(
        &self,
        call: &ModelToolCall,
        tool_plan: &crate::AgentToolPlan,
    ) -> Result<AgentToolOutput, AgentError> {
        // A malformed search is the model's mistake: it reads which field
        // to fix, and the turn goes on.
        let Some(object) = call.arguments.as_object() else {
            return Ok(search_argument_error(
                "the arguments must be a JSON object with a `query` string",
            ));
        };
        let query = match object.get("query") {
            Some(serde_json::Value::String(query)) => query.as_str(),
            None | Some(serde_json::Value::Null) => {
                return Ok(search_argument_error("`query` is required"));
            }
            Some(_) => return Ok(search_argument_error("`query` must be a string")),
        };
        if query.trim().is_empty() || query.len() > 512 {
            return Ok(AgentToolOutput::model_visible_error(json!({
                "error": "tool search query must contain 1 to 512 bytes"
            })));
        }
        // The kernel already turned `5.0` and `"5"` into 5. A limit out of
        // range is clamped, and the result says so; anything else is the
        // default.
        let requested = object.get("limit").and_then(serde_json::Value::as_u64);
        let limit = requested.map_or(crate::DEFAULT_AGENT_TOOL_SEARCH_RESULTS, |value| {
            usize::try_from(value)
                .unwrap_or(usize::MAX)
                .clamp(1, crate::MAX_AGENT_TOOL_SEARCH_RESULTS)
        });
        let note = requested
            .filter(|requested| *requested != limit as u64)
            .map(|requested| format!("limit {requested} is out of range; {limit} was used"));
        let matches = tool_plan.search_deferred(query, limit);
        let selected_tools = matches
            .iter()
            .map(|entry| entry.selection.clone())
            .collect::<Vec<_>>();
        let tool_search_results = crate::tool_catalog::coalesce_search_specs(
            matches
                .iter()
                .map(|entry| entry.specification.clone())
                .collect::<Vec<_>>(),
        );
        let visible = matches
            .iter()
            .map(|entry| {
                json!({
                    "name": entry.model_name,
                    "description": entry.description,
                    "scoreMilli": entry.score_milli,
                })
            })
            .collect::<Vec<_>>();
        Ok(AgentToolOutput {
            output: hanafubuki_agent_tool_contracts::ToolOutput::structured(
                match note {
                    Some(note) => json!({ "query": query, "tools": visible, "note": note }),
                    None => json!({ "query": query, "tools": visible }),
                },
                false,
            ),
            prepared_images: Vec::new(),
            selected_tools,
            tool_search_results,
            skill_context: None,
            display_metadata: Default::default(),
        })
    }
}

fn search_argument_error(message: &str) -> AgentToolOutput {
    AgentToolOutput::model_visible_error(json!({
        "error": format!("invalid tool_search arguments: {message}")
    }))
}
