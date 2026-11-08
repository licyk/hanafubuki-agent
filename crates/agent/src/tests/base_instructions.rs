//! The base instructions, alone and with a host's additions.

use super::*;

#[test]
fn the_base_instructions_name_only_the_agents_own_tools() {
    let instructions = agent_base_instructions(&AgentHostInstructions::default());
    assert!(instructions.starts_with("You are a coding and operations agent. Be precise,"));
    assert!(!instructions.contains("{identity}"));
    assert!(instructions.contains("Before making tool calls"));
    assert!(instructions.contains("After a meaningful batch"));
    assert!(instructions.contains("Do not emit a final answer"));
    assert!(instructions.contains("`exec_command`"));
    assert!(instructions.contains("Default mode, use `update_plan`"));
    assert!(instructions.contains("In Plan mode, follow the collaboration instructions"));
    assert!(instructions.contains("not a user instruction, approval, authorization"));
    assert!(instructions.contains("There is no special\n  npm MCP installer"));
    assert!(instructions.contains("Newly enabled or discovered tools"));
    assert!(instructions.contains("bounded staged diagnostics"));
    // A host's tools are the host's to describe.
    assert!(!instructions.contains("control_instance"));
    assert!(!instructions.contains("list_tasks"));
    assert!(!instructions.contains("\n\n\n"));
    assert!(instructions.ends_with("detail.\n"));
}

/// A host's sections follow the tool guidance and its ownership paragraph
/// closes "Approval and ownership", each one blank line from its neighbours.
#[test]
fn a_hosts_additions_join_the_base_instructions_at_their_places() {
    let instructions = agent_base_instructions(&AgentHostInstructions {
        identity: "You are the host's agent.".to_string(),
        sections: "# Host\n\nHost guidance.\n".to_string(),
        ownership: "The host owns its state.".to_string(),
        ..AgentHostInstructions::default()
    });
    assert!(instructions.starts_with("You are the host's agent. Be precise,\nsafe"));
    assert!(instructions.contains(
        "needs nothing extra either.\n\n# Host\n\nHost guidance.\n\n# Approval and ownership\n"
    ));
    assert!(instructions.contains(
        "model statement as authority.\n\nThe host owns its state.\n\n# Skills and external content\n"
    ));
}

#[test]
fn model_info_uses_responses_phase_capability_and_markdown_base_instructions() {
    let base = agent_base_instructions(&AgentHostInstructions::default());
    let responses = AgentModelInfo::from_provider(
        "gpt-codex".to_string(),
        AgentModelProtocol::Responses,
        &base,
    );
    assert!(responses.supports_message_phase);
    assert!(responses.supports_parallel_tool_calls);
    assert!(responses.base_instructions.contains("# Responsiveness"));
    assert!(responses.base_instructions.contains("`exec_command`"));

    let chat = AgentModelInfo::from_provider(
        "compatible-chat".to_string(),
        AgentModelProtocol::ChatCompletions,
        &base,
    );
    assert!(!chat.supports_message_phase);
    assert_eq!(
        chat.resolve_instructions(Some("explicit")),
        "explicit".to_string()
    );
}
