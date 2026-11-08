use super::*;

/// The base instructions, in the order they are joined; a host's additions
/// ([`AgentHostInstructions`]) go between them.
const BASE_INSTRUCTIONS_OPENING: &str = include_str!("prompts/base.md");
const BASE_INSTRUCTIONS_APPROVAL: &str = include_str!("prompts/approval.md");
const BASE_INSTRUCTIONS_CLOSING: &str = include_str!("prompts/closing.md");

/// Who the Agent is and what a host adds to its base instructions. The
/// default is the standalone Agent's: its own identity and name, no
/// additions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHostInstructions {
    /// The sentence that opens the base instructions (`You are …`).
    pub identity: String,
    /// The application the environment section names.
    pub application: String,
    /// Markdown sections after the tool guidance, before "Approval and
    /// ownership": how the host's tools and context are used. Empty adds
    /// nothing, as for `ownership`.
    pub sections: String,
    /// The paragraph that closes "Approval and ownership": what the host
    /// owns.
    pub ownership: String,
}

impl Default for AgentHostInstructions {
    fn default() -> Self {
        Self {
            identity: "You are a coding and operations agent.".to_string(),
            application: "hanafubuki-agent".to_string(),
            sections: String::new(),
            ownership: String::new(),
        }
    }
}

/// The base instructions with `host`'s additions, blocks separated by one
/// blank line.
pub(crate) fn agent_base_instructions(host: &AgentHostInstructions) -> String {
    let opening = BASE_INSTRUCTIONS_OPENING.replacen("{identity}", &host.identity, 1);
    let blocks = [
        &opening,
        &host.sections,
        BASE_INSTRUCTIONS_APPROVAL,
        &host.ownership,
        BASE_INSTRUCTIONS_CLOSING,
    ];
    let mut instructions = blocks
        .iter()
        .map(|block| block.trim_end())
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    instructions.push('\n');
    instructions
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentModelInfo {
    pub slug: String,
    pub base_instructions: String,
    pub instructions_template: Option<String>,
    pub supports_message_phase: bool,
    pub supports_parallel_tool_calls: bool,
}

impl AgentModelInfo {
    pub(crate) fn from_provider(
        slug: String,
        protocol: AgentModelProtocol,
        base_instructions: &str,
    ) -> Self {
        Self {
            slug,
            base_instructions: base_instructions.to_string(),
            instructions_template: None,
            supports_message_phase: protocol == AgentModelProtocol::Responses,
            supports_parallel_tool_calls: true,
        }
    }

    pub(crate) fn resolve_instructions(&self, explicit_override: Option<&str>) -> String {
        explicit_override
            .map(str::to_owned)
            .or_else(|| self.instructions_template.clone())
            .unwrap_or_else(|| self.base_instructions.clone())
    }
}

pub(crate) fn model_api_protocol(
    protocol: AgentModelProtocol,
) -> hanafubuki_agent_kernel::ModelApiProtocol {
    match protocol {
        AgentModelProtocol::Responses => hanafubuki_agent_kernel::ModelApiProtocol::Responses,
        AgentModelProtocol::ChatCompletions => {
            hanafubuki_agent_kernel::ModelApiProtocol::ChatCompletions
        }
        AgentModelProtocol::AnthropicMessages => {
            hanafubuki_agent_kernel::ModelApiProtocol::AnthropicMessages
        }
    }
}

pub(crate) fn model_compaction_capability(
    capability: hanafubuki_agent_config::AgentRemoteCompactionCapability,
) -> ModelCompactionCapability {
    match capability {
        hanafubuki_agent_config::AgentRemoteCompactionCapability::Disabled => {
            ModelCompactionCapability::LocalSummary
        }
        hanafubuki_agent_config::AgentRemoteCompactionCapability::ResponsesCompact => {
            ModelCompactionCapability::ResponsesCompact
        }
        hanafubuki_agent_config::AgentRemoteCompactionCapability::ResponsesCompactV2 => {
            ModelCompactionCapability::ResponsesCompactV2
        }
    }
}
