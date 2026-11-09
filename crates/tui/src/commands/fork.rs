// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/fork [name]`: a copy of the session as it stands, shown from then on
//! and named when a name is given; the original stays as it was. Where a
//! worktree can be made (a Git repository, the `worktree` plugin running),
//! it first asks where the fork runs: here, or in a new worktree, as
//! `/worktree`'s "Continue this conversation" (Codex's session checkout
//! picker).

use super::worktree::enter;
use super::{send_session_call, CommandContext, CommandSpec};
use crate::agent_bridge::{SessionCall, SessionReply};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
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

pub(super) fn on_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    match reply {
        SessionReply::Forked(name) => {
            let message = match name {
                Some(name) => format!("Forked into a new session, \"{name}\""),
                None => "Forked into a new session".to_string(),
            };
            context.add_to_history(history_cell::new_info_event(message, None));
        }
        SessionReply::ForkChoice(name) => show_choice(context, name),
        _ => {}
    }
}

fn show_choice(context: &mut CommandContext<'_>, name: Option<String>) {
    let here = name.clone();
    context.show_selection_view(SelectionViewParams {
        title: Some("Where should the fork run?".to_string()),
        items: vec![
            SelectionItem {
                name: "Fork here".to_string(),
                description: Some("Keep the current working directory".to_string()),
                actions: vec![Box::new(move |tx| {
                    send_session_call(tx, SessionCall::ForkHere(here.clone()))
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Fork into a new worktree".to_string(),
                description: Some(
                    "A separate checkout of this repository; the fork goes on there".to_string(),
                ),
                actions: vec![Box::new(move |tx| {
                    send_session_call(tx, enter(true, name.clone()))
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
}
