// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The commands that report what the Agent has: `/skills`, `/status` and
//! `/mcp`. The bridge asks the facade; the chat writes what comes back.

use super::CommandSpec;
use crate::app_event::AgentCommand;

pub(super) static SKILLS: CommandSpec = CommandSpec {
    name: "skills",
    aliases: &[],
    description: "list the Skills the Agent can use",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.agent_command(AgentCommand::Skills),
};

pub(super) static STATUS: CommandSpec = CommandSpec {
    name: "status",
    aliases: &[],
    description: "show current session configuration and token usage",
    inline_args: false,
    during_task: true,
    needs_session: true,
    dispatch: |context, _args| context.agent_command(AgentCommand::Status),
};

pub(super) static MCP: CommandSpec = CommandSpec {
    name: "mcp",
    aliases: &[],
    description: "list configured MCP servers and their tools",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.agent_command(AgentCommand::Mcp),
};
