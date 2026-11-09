// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/fork [name]`: a copy of the session as it stands, shown from then on
//! and named when a name is given; the original stays as it was.

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::history_cell;

pub(super) static FORK: CommandSpec = CommandSpec {
    name: "fork",
    aliases: &[],
    description: "fork the current chat into a new session",
    inline_args: true,
    during_task: false,
    needs_session: false,
    dispatch: |context, args| {
        let name = Some(args.trim().to_string()).filter(|name| !name.is_empty());
        context.session_call(SessionCall::Fork(name));
    },
};

pub(super) fn on_reply(context: &mut CommandContext<'_>, name: Option<String>) {
    let message = match name {
        Some(name) => format!("Forked into a new session, \"{name}\""),
        None => "Forked into a new session".to_string(),
    };
    context.add_to_history(history_cell::new_info_event(message, None));
}
