// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/pwd` (or `/cwd`): the workspace roots, the first the working
//! directory relative paths are read against.

use std::path::PathBuf;

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::history_cell;

pub(super) static PWD: CommandSpec = CommandSpec {
    name: "pwd",
    aliases: &["cwd"],
    description: "show the current working directory",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.session_call(SessionCall::Roots),
};

pub(super) fn on_reply(context: &mut CommandContext<'_>, roots: Vec<PathBuf>) {
    let mut roots = roots.into_iter();
    let Some(first) = roots.next() else {
        let message = "There is no workspace root".to_string();
        return context.add_to_history(history_cell::new_error_event(message));
    };
    context.add_to_history(history_cell::new_info_event(
        format!("Current working directory: {}", first.display()),
        None,
    ));
    for root in roots {
        context.add_to_history(history_cell::new_info_event(
            format!("Also in the workspace: {}", root.display()),
            None,
        ));
    }
}
