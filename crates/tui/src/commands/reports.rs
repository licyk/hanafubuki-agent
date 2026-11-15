// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The commands that report what the Agent has: `/skills` and `/status`
//! (`/mcp verbose` is `/mcp`'s). The bridge asks the facade; the chat
//! writes what comes back.

use super::CommandSpec;
use crate::app_event::AgentCommand;

pub(super) static SKILLS: CommandSpec = CommandSpec {
    name: "skills",
    aliases: &[],
    description: "manage Skills, install from a directory, and inspect their source",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch: |context, args| {
        if args.trim() == "verbose" {
            context.agent_command(AgentCommand::Skills);
        } else {
            super::open_settings(context, "skills");
        }
    },
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
