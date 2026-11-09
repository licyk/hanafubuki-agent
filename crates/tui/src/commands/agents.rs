// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/agents`: the command center (Codex's agents overview): the sessions,
//! the latest first, each with its children, to open, start, fork,
//! archive, delete or interrupt. The app shows it (`agents_center`).

use super::{CommandContext, CommandSpec};
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;

pub(super) static AGENTS: CommandSpec = CommandSpec {
    name: "agents",
    aliases: &[],
    description: "open the command center: the sessions and their subagents",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    context.send(AppEvent::Workspace(WorkspaceEvent::OpenAgents));
}
