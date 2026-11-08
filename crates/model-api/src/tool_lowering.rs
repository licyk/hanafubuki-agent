use std::borrow::Cow;

use hanafubuki_agent_tool_contracts::ToolSpec;
use serde_json::Value;

use crate::{ModelError, ModelResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoweredFunctionTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub strict: bool,
}

pub(crate) fn lower_function_tools(
    tools: &[ToolSpec],
    protocol: &'static str,
) -> ModelResult<Vec<LoweredFunctionTool>> {
    let mut lowered = Vec::new();
    for tool in tools {
        match tool {
            ToolSpec::Function(tool) => {
                if tool.deferred {
                    return Err(unsupported(protocol, "deferred function tools"));
                }
                lowered.push(LoweredFunctionTool {
                    name: tool.name.flattened_alias(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                    strict: tool.strict,
                });
            }
            ToolSpec::Namespace(_) => return Err(unsupported(protocol, "namespace tools")),
            ToolSpec::ToolSearch(_) => return Err(unsupported(protocol, "native tool search")),
            ToolSpec::HostedWebSearch(_) => return Err(unsupported(protocol, "hosted web search")),
            ToolSpec::Freeform(_) => return Err(unsupported(protocol, "freeform tools")),
        }
    }
    Ok(lowered)
}

fn unsupported(protocol: &'static str, capability: &str) -> ModelError {
    ModelError::InvalidRequest(format!(
        "{protocol} does not support {capability}; the tool plan must omit or lower it"
    ))
}

/// A call's name flattened to one string: `namespace__name` for a call made
/// in a native namespace. It is how a protocol without namespaces replays a
/// history call recorded under one (a session that moved from Responses),
/// and how evidence and logs name it; a returned call is never resolved
/// through it.
pub fn flattened_tool_call_name<'a>(namespace: Option<&str>, name: &'a str) -> Cow<'a, str> {
    match namespace {
        Some(namespace) => Cow::Owned(format!("{namespace}__{name}")),
        None => Cow::Borrowed(name),
    }
}
