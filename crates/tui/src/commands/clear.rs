// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/clear [name]`: the terminal's scrollback and screen are cleared (in
//! fullscreen, the transcript) and the interface shows no session, as
//! after `/new`; the session the next message creates gets the name.

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::app_event::{AppCommand, AppEvent};

pub(super) static CLEAR: CommandSpec = CommandSpec {
    name: "clear",
    aliases: &[],
    description: "clear the terminal and start a new chat",
    inline_args: true,
    during_task: false,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let name = Some(args.trim().to_string()).filter(|name| !name.is_empty());
    context.send(AppEvent::Command(AppCommand::ClearScreen));
    context.session_call(SessionCall::Clear(name));
}
