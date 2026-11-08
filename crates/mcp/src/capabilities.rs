use serde::{Deserialize, Serialize};

use crate::McpConnectionGeneration;

pub const MAX_MCP_INSTRUCTIONS_BYTES: usize = 4 * 1024;
pub const MAX_MCP_INSTRUCTIONS_SUMMARY_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilitySnapshot {
    pub connection_generation: McpConnectionGeneration,
    pub protocol_version: String,
    pub implementation_name: String,
    pub implementation_version: String,
    pub tools: bool,
    pub tools_list_changed: bool,
    pub resources: bool,
    pub resources_subscribe: bool,
    pub resources_list_changed: bool,
    pub prompts: bool,
    pub prompts_list_changed: bool,
    pub logging: bool,
    pub completions: bool,
    pub tasks_declared: bool,
    pub tasks_wired: bool,
    pub elicitation_wired: bool,
    pub instructions_present: bool,
    pub instructions_bytes: usize,
    pub instructions_summary: Option<String>,
    pub stale: bool,
}

impl McpCapabilitySnapshot {
    pub fn from_peer(
        generation: McpConnectionGeneration,
        info: &rmcp::model::ServerPeerInfo,
    ) -> Self {
        let instructions = info.instructions.as_deref();
        let implementation = info.server_info.as_ref();
        let tools = info.capabilities.tools.as_ref();
        let resources = info.capabilities.resources.as_ref();
        let prompts = info.capabilities.prompts.as_ref();
        Self {
            connection_generation: generation,
            protocol_version: info.protocol_version.to_string(),
            implementation_name: implementation
                .map(|value| bounded(&value.name, 256))
                .unwrap_or_default(),
            implementation_version: implementation
                .map(|value| bounded(&value.version, 256))
                .unwrap_or_default(),
            tools: tools.is_some(),
            tools_list_changed: tools.and_then(|value| value.list_changed).unwrap_or(false),
            resources: resources.is_some(),
            resources_subscribe: resources.and_then(|value| value.subscribe).unwrap_or(false),
            resources_list_changed: resources
                .and_then(|value| value.list_changed)
                .unwrap_or(false),
            prompts: prompts.is_some(),
            prompts_list_changed: prompts
                .and_then(|value| value.list_changed)
                .unwrap_or(false),
            logging: info.capabilities.logging.is_some(),
            completions: info.capabilities.completions.is_some(),
            tasks_declared: info.capabilities.supports_tasks(),
            tasks_wired: false,
            elicitation_wired: true,
            instructions_present: instructions.is_some(),
            instructions_bytes: instructions.map_or(0, str::len),
            instructions_summary: instructions.map(|value| {
                bounded(
                    &bounded(value, MAX_MCP_INSTRUCTIONS_BYTES),
                    MAX_MCP_INSTRUCTIONS_SUMMARY_BYTES,
                )
            }),
            stale: false,
        }
    }

    pub fn stale(mut self) -> Self {
        self.stale = true;
        self
    }
}

fn bounded(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_string();
    }
    let boundary = value
        .char_indices()
        .take_while(|(index, _)| *index <= maximum)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    value[..boundary].to_string()
}

#[cfg(test)]
mod tests;
