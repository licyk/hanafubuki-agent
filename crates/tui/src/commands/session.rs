// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The commands over the session itself: `/new` and `/compact`.

use super::CommandSpec;
use crate::app_event::AgentCommand;

pub(super) static NEW: CommandSpec = CommandSpec {
    name: "new",
    aliases: &[],
    description: "start a new chat during a conversation",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch: |context, _args| context.agent_command(AgentCommand::NewSession),
};

pub(super) static COMPACT: CommandSpec = CommandSpec {
    name: "compact",
    aliases: &[],
    description: "summarize conversation to prevent hitting the context limit",
    inline_args: false,
    during_task: false,
    needs_session: true,
    dispatch: |context, _args| context.agent_command(AgentCommand::Compact),
};
