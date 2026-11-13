//! `/context`: what the session's last request held, group by group, and
//! the prompt cache, read through the facade's `session_context`. It reads
//! a snapshot, so it runs during a turn too.

use super::CommandSpec;
use crate::app_event::AgentCommand;

pub(super) static CONTEXT: CommandSpec = CommandSpec {
    name: "context",
    aliases: &[],
    description: "show what the context holds: the last request by group, and the prompt cache",
    inline_args: false,
    during_task: true,
    needs_session: true,
    dispatch: |context, _args| context.agent_command(AgentCommand::Context),
};
