// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! `/exit`, and `/quit` with it.

use super::CommandSpec;
use crate::app_event::AppEvent;

pub(super) static EXIT: CommandSpec = CommandSpec {
    name: "exit",
    aliases: &["quit"],
    description: "exit the Agent",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.send(AppEvent::Exit),
};
