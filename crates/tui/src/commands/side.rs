// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/side [message]` (`/btw`): the side conversation of the session, a
//! quick question beside the main one (Codex's `app/side.rs`). It opens
//! (or the open one is shown again) and the message, when there is one,
//! is its next. `/side close` closes it. The app does the work
//! (`app::workspace`): in fullscreen the side conversation is a pane
//! beside the transcript, in scrollback mode the interface switches to it.

use super::{CommandContext, CommandSpec};
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;

/// The commands a second conversation (the side conversation, a subagent
/// shown) runs: Codex's `available_in_side_conversation`, those this
/// interface has, and `/side` itself to close it.
pub(crate) const SECOND_CONVERSATION_COMMANDS: &[&str] = &[
    "copy", "agents", "export", "raw", "diff", "mention", "status", "warnings", "pwd", "side",
];

pub(super) static SIDE: CommandSpec = CommandSpec {
    name: "side",
    aliases: &["btw"],
    description: "ask a quick side question without derailing the conversation",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let event = match args.trim() {
        "close" => WorkspaceEvent::CloseSide,
        "" => WorkspaceEvent::OpenSide(None),
        message => WorkspaceEvent::OpenSide(Some(message.to_string())),
    };
    context.send(AppEvent::Workspace(event));
}
